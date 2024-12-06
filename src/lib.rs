#![allow(deprecated)]
#![warn(rust_2018_idioms)]
#![warn(missing_debug_implementations)]
#![warn(missing_docs)]

//! A simple WebRTC streaming server. It streams video and audio from a file to a browser client.

use anyhow::Result;
use hyper::body::HttpBody;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use lazy_static::lazy_static;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

lazy_static! {
  static ref CONN_DESC_CHANNEL: Arc<Mutex<Option<mpsc::Sender<RTCSessionDescription>>>> =
    Arc::new(Mutex::new(None));
  static ref CONN_DESC_CHANNEL_CALLBACK: Arc<Mutex<Option<mpsc::Receiver<String>>>> =
    Arc::new(Mutex::new(None));
}

async fn remote_handler(mut req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
  match (req.method(), req.uri().path()) {
    (&Method::GET, "/") => {
      let response = Response::new(include_str!("../webrtc.html").into());
      Ok(response)
    }
    (&Method::POST, "/local") => {
      if let Ok(desc) = req.body_mut().collect().await {
        let desc = desc.to_bytes().to_vec();
        let desc = String::from_utf8_lossy(&desc);
        let sender = CONN_DESC_CHANNEL.lock().await;
        let sender = sender.as_ref().unwrap();
        let desc_data = decode(&desc).expect("Invalid base64 encoded description");
        let offer = serde_json::from_str::<RTCSessionDescription>(&desc_data)
          .expect("Invalid session description");
        sender.send(offer).await.expect("Receiver dropped");
        let mut receiver = CONN_DESC_CHANNEL_CALLBACK.lock().await;
        let receiver = receiver.as_mut().unwrap();
        if let Some(desc) = receiver.recv().await {
          return Ok(Response::new(desc.into()));
        }
      }
      let mut response = Response::default();
      *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
      Ok(response)
    }
    _ => {
      let mut not_found = Response::default();
      *not_found.status_mut() = StatusCode::NOT_FOUND;
      Ok(not_found)
    }
  }
}

/// Bind to a port and serve.
pub async fn http_server(
  port: u16,
) -> (mpsc::Receiver<RTCSessionDescription>, mpsc::Sender<String>) {
  let (conn_chan_tx, conn_chan_rx) = mpsc::channel::<RTCSessionDescription>(1);
  let (callback_chan_tx, callback_chan_rx) = mpsc::channel::<String>(1);
  {
    let mut tx = CONN_DESC_CHANNEL.lock().await;
    *tx = Some(conn_chan_tx);
  }
  {
    let mut tx = CONN_DESC_CHANNEL_CALLBACK.lock().await;
    *tx = Some(callback_chan_rx);
  }

  tokio::spawn(async move {
    let addr = SocketAddr::from_str(&format!("0.0.0.0:{port}")).unwrap();
    let service = make_service_fn(|_| async { Ok::<_, hyper::Error>(service_fn(remote_handler)) });
    let server = Server::bind(&addr).serve(service);

    if let Err(e) = server.await {
      eprintln!("server error: {e}");
    }
  });

  (conn_chan_rx, callback_chan_tx)
}

/// Helper function to read input base64 data.
pub fn must_read_stdin() -> Result<String> {
  let mut line = String::new();

  std::io::stdin().read_line(&mut line)?;
  line = line.trim().to_owned();
  println!();

  Ok(line)
}

/// Encode base64 wrapper function.
pub fn encode(b: &str) -> String {
  base64::encode(b.as_bytes())
}

/// Decode base64 wrapper function.
pub fn decode(s: &str) -> Result<String> {
  let b = base64::decode(s.as_bytes())?;
  let s = String::from_utf8(b)?;
  Ok(s)
}
