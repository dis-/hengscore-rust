#![forbid(unsafe_code)]
use crate::{
    registration::{Fault, RegistrationError, Store},
    submission::{MAX_RAW, parse_submission, validate_content_type},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{sync::Semaphore, time::timeout};

#[derive(Clone)]
struct App {
    root: PathBuf,
    permit: Arc<Semaphore>,
    // Test-only startup option. HTTP requests cannot select faults.
    fault: Option<String>,
    store_id: Option<String>,
}

fn json(status: StatusCode, body: String) -> Response {
    let mut response = (status, body).into_response();
    let headers = response.headers_mut();
    for (name, value) in [
        ("content-type", "application/json; charset=utf-8"),
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("connection", "close"),
    ] {
        headers.insert(name, value.parse().expect("literal header"));
    }
    if status == StatusCode::SERVICE_UNAVAILABLE {
        headers.insert("retry-after", "1".parse().expect("literal header"));
    }
    response
}

fn error(status: StatusCode, code: &str) -> Response {
    // Codes are literals, never database errors or submitted text.
    json(status, format!("{{\"error\":\"{code}\"}}\n"))
}

fn one_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, ()> {
    let mut values = headers.get_all(name).iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(());
    }
    first.to_str().map(Some).map_err(|_| ())
}

async fn health(request: Request<Body>) -> Response {
    if request.method() != Method::GET {
        return method_not_allowed("GET");
    }
    json(StatusCode::OK, "{\"status\":\"ready\"}\n".into())
}

fn method_not_allowed(allow: &'static str) -> Response {
    let mut response = error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed");
    response
        .headers_mut()
        .insert("allow", allow.parse().expect("literal method"));
    response
}

async fn post(State(app): State<App>, request: Request<Body>) -> Response {
    if request.method() != Method::POST {
        return method_not_allowed("POST");
    }
    if request.uri().query().is_some() {
        return error(StatusCode::BAD_REQUEST, "unexpected_query");
    }
    let headers = request.headers();
    if headers.len() > 64
        || headers
            .iter()
            .map(|(k, v)| k.as_str().len() + v.len())
            .sum::<usize>()
            > 16 * 1024
    {
        return error(
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            "headers_too_large",
        );
    }
    let header = match one_header(headers, "content-type") {
        Ok(Some(value)) if validate_content_type(value).is_ok() => value.to_owned(),
        Err(()) => return error(StatusCode::BAD_REQUEST, "duplicate_or_invalid_header"),
        _ => {
            return error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_content_type",
            );
        }
    };
    match one_header(headers, "content-encoding") {
        Ok(None | Some("identity")) => (),
        _ => {
            return error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "content_encoding_not_supported",
            );
        }
    }
    let token = match one_header(headers, "idempotency-key") {
        Ok(None) => None,
        Ok(Some(value))
            if !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') =>
        {
            Some(value.to_owned())
        }
        _ => return error(StatusCode::BAD_REQUEST, "invalid_idempotency_key"),
    };
    if headers.contains_key("transfer-encoding") && headers.contains_key("content-length") {
        return error(StatusCode::BAD_REQUEST, "ambiguous_body_framing");
    }
    match one_header(headers, "content-length") {
        Ok(Some(value)) => match value.parse::<u64>() {
            Ok(size) if size <= MAX_RAW as u64 => (),
            Ok(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large"),
            Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_content_length"),
        },
        Ok(None) => (),
        Err(()) => return error(StatusCode::BAD_REQUEST, "duplicate_content_length"),
    }
    // No unbounded queue of bodies or blocking SQLite workers.
    let permit = match app.permit.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "busy"),
    };
    let raw = match timeout(
        Duration::from_secs(2),
        to_bytes(request.into_body(), MAX_RAW),
    )
    .await
    {
        Err(_) => return error(StatusCode::REQUEST_TIMEOUT, "body_timeout"),
        Ok(Err(error_)) => {
            use std::error::Error;
            let mut cause: &(dyn Error + 'static) = &error_;
            loop {
                if cause.is::<http_body_util::LengthLimitError>() {
                    return error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large");
                }
                match cause.source() {
                    Some(next) => cause = next,
                    None => break,
                }
            }
            return error(StatusCode::BAD_REQUEST, "invalid_body");
        }
        Ok(Ok(bytes)) => bytes,
    };
    enum Outcome {
        Registered(i64),
        Invalid,
        Reference,
        Conflict,
        Unavailable,
    }
    let keyed = token.is_some();
    let task = tokio::task::spawn_blocking(move || {
        // Permit remains held even if the HTTP future is cancelled or times out.
        let _permit = permit;
        if parse_submission(&header, &raw).is_err() {
            return Outcome::Invalid;
        }
        let opened = match app.store_id.as_deref() {
            Some(id) => Store::open_managed_nonblocking(&app.root, id),
            None => Store::open_nonblocking(&app.root),
        };
        let result = opened.and_then(|mut store| match token {
            Some(token) => store.register(&token, &header, &raw, &Fault(app.fault)),
            None => store.register_unkeyed(&header, &raw, &Fault(app.fault)),
        });
        match result {
            Ok(id) => Outcome::Registered(id),
            Err(error) => match error.downcast_ref::<RegistrationError>() {
                Some(RegistrationError::InvalidReference) => Outcome::Reference,
                Some(RegistrationError::IdempotencyConflict) => Outcome::Conflict,
                None => Outcome::Unavailable,
            },
        }
    });
    match timeout(Duration::from_secs(5), task).await {
        Ok(Ok(Outcome::Registered(id))) => json(
            StatusCode::OK,
            format!(
                "{{\"status\":\"registered\",\"score_id\":{id},\"retry_mode\":\"{}\"}}\n",
                if keyed {
                    "idempotency_key"
                } else {
                    "legacy_unkeyed"
                }
            ),
        ),
        Ok(Ok(Outcome::Invalid)) => error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_submission"),
        Ok(Ok(Outcome::Reference)) => error(StatusCode::UNPROCESSABLE_ENTITY, "unknown_reference"),
        Ok(Ok(Outcome::Conflict)) => error(StatusCode::CONFLICT, "idempotency_conflict"),
        _ => error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    }
}

/// Shared submission routes. The caller must recover an explicitly marked
/// disposable Store before exposing the router, and bind only to loopback.
pub fn router(root: PathBuf, fault: Fault) -> Router {
    make_router(root, fault.0, None)
}

/// Managed service: no HTTP input or startup argument can inject test faults.
pub fn managed_router(root: PathBuf, store_id: String) -> Router {
    make_router(root, None, Some(store_id))
}

fn make_router(root: PathBuf, fault: Option<String>, store_id: Option<String>) -> Router {
    Router::new()
        .route("/scores", any(post))
        .route("/healthz", any(health))
        .route("/register_score.php", any(post))
        .route("/heng_score/register_score.php", any(post))
        .route("/~dis/heng_score/register_score.php", any(post))
        .with_state(App {
            root,
            permit: Arc::new(Semaphore::new(1)),
            fault,
            store_id,
        })
}
pub async fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "not_found")
}
