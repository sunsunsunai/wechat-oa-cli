use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use super::*;

async fn test_server(body: &'static str) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let request_count = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&request_count);

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            count.fetch_add(1, Ordering::SeqCst);
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });

    (format!("http://{address}/cgi-bin/token"), request_count)
}

#[tokio::test]
async fn returns_and_caches_token() {
    let (endpoint, request_count) =
        test_server(r#"{"access_token":"token-1","expires_in":7200}"#).await;
    let manager = TokenManager::with_options(
        "app-id".to_owned(),
        "secret".to_owned(),
        endpoint,
        Duration::from_secs(300),
    );
    let http = reqwest::Client::new();

    assert_eq!(manager.access_token(&http).await.unwrap(), "token-1");
    assert_eq!(manager.access_token(&http).await.unwrap(), "token-1");
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn reports_wechat_api_error() {
    let (endpoint, _) = test_server(r#"{"errcode":40013,"errmsg":"invalid appid"}"#).await;
    let manager = TokenManager::with_options(
        "bad-app-id".to_owned(),
        "secret".to_owned(),
        endpoint,
        Duration::ZERO,
    );

    let error = manager
        .access_token(&reqwest::Client::new())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        WechatError::Api(WechatApiError {
            code: 40013,
            ref message,
            ..
        }) if message == "invalid appid"
    ));
}

#[tokio::test]
async fn invalidates_only_the_rejected_cached_token() {
    let (endpoint, _) = test_server(r#"{"access_token":"token-1","expires_in":7200}"#).await;
    let manager = TokenManager::with_options(
        "app-id".to_owned(),
        "secret".to_owned(),
        endpoint,
        Duration::from_secs(300),
    );
    let http = reqwest::Client::new();

    manager.access_token(&http).await.unwrap();
    manager.invalidate("a-different-token").await;
    assert_eq!(manager.cached_token().await.as_deref(), Some("token-1"));

    manager.invalidate("token-1").await;
    assert_eq!(manager.cached_token().await, None);
}

#[tokio::test]
async fn concurrent_requests_share_one_refresh() {
    let (endpoint, request_count) =
        test_server(r#"{"access_token":"shared-token","expires_in":7200}"#).await;
    let manager = Arc::new(TokenManager::with_options(
        "app-id".to_owned(),
        "secret".to_owned(),
        endpoint,
        Duration::from_secs(300),
    ));
    let http = reqwest::Client::new();

    let calls = (0..8).map(|_| {
        let manager = Arc::clone(&manager);
        let http = http.clone();
        tokio::spawn(async move { manager.access_token(&http).await.unwrap() })
    });

    for call in calls {
        assert_eq!(call.await.unwrap(), "shared-token");
    }
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
}
