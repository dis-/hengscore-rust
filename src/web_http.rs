#![forbid(unsafe_code)]
use crate::web::{self, PageError, Theme};
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
    if request.uri().path() == "/theme" {
        return match web::theme_switch(request.uri().query()) {
            Ok((theme, back)) => {
                let mut r = response(StatusCode::SEE_OTHER, String::new());
                for (k, v) in [("location", back), ("set-cookie", theme.set_cookie())] {
                    r.headers_mut()
                        .insert(k, v.parse().expect("validated header"));
                }
                r
            }
            Err(e) => response(StatusCode::BAD_REQUEST, web::error_page(&e)),
        };
    }
    let theme = Theme::from_cookie(
        request
            .headers()
            .get("cookie")
            .and_then(|v| v.to_str().ok()),
    );
    let here = request
        .uri()
        .path_and_query()
        .map_or_else(|| "/".to_owned(), |p| p.as_str().to_owned());
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
            let mut r = response(StatusCode::OK, web::apply_theme(html, theme, &here));
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
            response(status, web::apply_theme(web::error_page(&e), theme, &here))
        }
    }
}
pub fn router(root: PathBuf) -> Router {
    Router::new().fallback(page).with_state(App {
        root,
        permit: Arc::new(Semaphore::new(4)),
    })
}
