#![forbid(unsafe_code)]
use crate::web::{self, PageError};
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Semaphore;
#[derive(Clone)]
struct App {
    root: PathBuf,
    permit: Arc<Semaphore>,
}
fn response(status: StatusCode, html: String) -> Response {
    let mut r = (status, html).into_response();
    for (k, v) in [
        ("content-type", "text/html; charset=utf-8"),
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("connection", "close"),
        (
            "content-security-policy",
            "default-src 'none'; frame-src 'self'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
        ),
        ("referrer-policy", "no-referrer"),
    ] {
        r.headers_mut()
            .insert(k, v.parse().expect("literal header"));
    }
    r
}
async fn page(State(app): State<App>, request: Request<Body>) -> Response {
    if request.method() != axum::http::Method::GET {
        let mut r = response(
            StatusCode::METHOD_NOT_ALLOWED,
            web::error_page(&PageError::BadRequest),
        );
        r.headers_mut()
            .insert("allow", "GET".parse().expect("literal header"));
        return r;
    }
    if request.uri().to_string().len() > 16384 {
        return response(
            StatusCode::BAD_REQUEST,
            web::error_page(&PageError::BadRequest),
        );
    }
    let permit = match app.permit.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            return response(
                StatusCode::SERVICE_UNAVAILABLE,
                web::error_page(&PageError::Unavailable),
            );
        }
    };
    let path = request.uri().path().to_owned();
    let is_feed = path == "/feed/newcome-atom.xml";
    let query = request.uri().query().map(str::to_owned);
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        web::render(&app.root, &path, query.as_deref())
    });
    match task.await {
        Ok(Ok(html)) => {
            let mut r = response(StatusCode::OK, html);
            if is_feed {
                r.headers_mut().insert(
                    "content-type",
                    "application/atom+xml; charset=utf-8"
                        .parse()
                        .expect("literal header"),
                );
            }
            r
        }
        error => {
            let e = match error {
                Ok(Err(e)) => e,
                _ => PageError::Unavailable,
            };
            let status = match e {
                PageError::BadRequest => StatusCode::BAD_REQUEST,
                PageError::NotFound => StatusCode::NOT_FOUND,
                PageError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
                PageError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            };
            response(status, web::error_page(&e))
        }
    }
}
pub fn router(root: PathBuf) -> Router {
    Router::new().fallback(page).with_state(App {
        root,
        permit: Arc::new(Semaphore::new(4)),
    })
}
