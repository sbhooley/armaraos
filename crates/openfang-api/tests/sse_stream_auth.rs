//! SSE stream auth: `/api/events/stream`, `/api/logs/stream`, and `/api/logs/daemon/stream` skip
//! credentials only on loopback. Remote clients must use `Authorization: Bearer` or `?token=` when
//! `api_key` is set.

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use openfang_kernel::OpenFangKernel;
use openfang_types::config::{DefaultModelConfig, KernelConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn events_stream_non_loopback_unauthorized_without_credentials() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: "sse-auth-secret-1".to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    let kernel = Arc::new(OpenFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, _) = openfang_api::server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;

    let mut req = Request::builder()
        .uri("/api/events/stream")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 4444))));

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    drop(tmp);
}

#[tokio::test]
async fn events_stream_non_loopback_ok_with_bearer() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: "sse-auth-secret-2".to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    let kernel = Arc::new(OpenFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, _) = openfang_api::server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;

    let mut req = Request::builder()
        .uri("/api/events/stream")
        .header("Authorization", "Bearer sse-auth-secret-2")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 2], 4444))));

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    drop(tmp);
}

#[tokio::test]
async fn events_stream_loopback_ok_without_bearer_when_api_key_set() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: "sse-auth-secret-3".to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    let kernel = Arc::new(OpenFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, _) = openfang_api::server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;

    let mut req = Request::builder()
        .uri("/api/events/stream")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    drop(tmp);
}

#[tokio::test]
async fn daemon_stream_non_loopback_unauthorized_without_credentials() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: "sse-daemon-secret-1".to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    let kernel = Arc::new(OpenFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, _) = openfang_api::server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;

    let mut req = Request::builder()
        .uri("/api/logs/daemon/stream")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 10], 4444))));

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    drop(tmp);
}

#[tokio::test]
async fn daemon_stream_loopback_ok_without_bearer_when_api_key_set() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: "sse-daemon-secret-2".to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    let kernel = Arc::new(OpenFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, _) = openfang_api::server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;

    let mut req = Request::builder()
        .uri("/api/logs/daemon/stream")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5556))));

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    drop(tmp);
}

#[tokio::test]
async fn logs_stream_non_loopback_unauthorized_without_credentials() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: "sse-logs-secret-1".to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    let kernel = Arc::new(OpenFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, _) = openfang_api::server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;

    let mut req = Request::builder()
        .uri("/api/logs/stream")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 20], 4444))));

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    drop(tmp);
}

#[tokio::test]
async fn logs_stream_non_loopback_ok_with_bearer() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: "sse-logs-secret-2".to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    let kernel = Arc::new(OpenFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, _) = openfang_api::server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;

    let mut req = Request::builder()
        .uri("/api/logs/stream")
        .header("Authorization", "Bearer sse-logs-secret-2")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 21], 4444))));

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    drop(tmp);
}

#[tokio::test]
async fn logs_stream_loopback_ok_without_bearer_when_api_key_set() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: "sse-logs-secret-3".to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    let kernel = Arc::new(OpenFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, _) = openfang_api::server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;

    let mut req = Request::builder()
        .uri("/api/logs/stream")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5560))));

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    drop(tmp);
}
