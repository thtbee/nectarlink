// SPDX-License-Identifier: GPL-3.0-or-later

use std::{convert::Infallible, time::Duration};

use anyhow::Result;
use bytes::{BufMut, Bytes, BytesMut};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode, body::Incoming, header, server::conn::http1, service::service_fn,
};
use hyper_util::rt::TokioIo;
use tokio::{net::TcpListener, task::JoinSet};

use super::{Route, Shared, transfer::CHUNK};

const PAGE: &[u8] = include_bytes!("../../anysend-web/site/index.html");
const SCRIPT: &[u8] = include_bytes!("../../anysend-web/site/receive.js");

pub(super) async fn serve(listener: TcpListener, session: Shared) -> Result<()> {
    let host = listener.local_addr()?.to_string();
    let origin = format!("http://{host}");
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept(), if connections.len() < 24 => {
                let (stream, _) = accepted?;
                let session = session.clone();
                let host = host.clone();
                let origin = origin.clone();
                connections.spawn(async move {
                    let service = service_fn(move |request| {
                        reply(request, session.clone(), host.clone(), origin.clone())
                    });
                    let connection = http1::Builder::new().serve_connection(TokioIo::new(stream), service);
                    let _ = tokio::time::timeout(Duration::from_secs(40), connection).await;
                });
            }
            _ = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

async fn reply(
    request: Request<Incoming>,
    session: Shared,
    host: String,
    origin: String,
) -> Result<Response<Full<Bytes>>, Infallible> {
    if request.headers().get(header::HOST).and_then(|value| value.to_str().ok()) != Some(host.as_str()) {
        return Ok(response(StatusCode::BAD_REQUEST, "text/plain", Bytes::new()));
    }
    let reply = match (request.method(), request.uri().path()) {
        (&Method::GET, "/") => response(StatusCode::OK, "text/html; charset=utf-8", Bytes::from_static(PAGE)),
        (&Method::GET, "/receive.js") => {
            response(StatusCode::OK, "application/javascript; charset=utf-8", Bytes::from_static(SCRIPT))
        }
        (&Method::POST, "/packet") => {
            if request
                .headers()
                .get(header::ORIGIN)
                .is_some_and(|value| value.as_bytes() != origin.as_bytes())
            {
                return Ok(response(StatusCode::FORBIDDEN, "text/plain", Bytes::new()));
            }
            let Ok(body) = Limited::new(request.into_body(), CHUNK + 33).collect().await else {
                return Ok(response(StatusCode::PAYLOAD_TOO_LARGE, "text/plain", Bytes::new()));
            };
            let packets = session.lock().await.handle(Route::Lan, &body.to_bytes()).await;
            let mut framed = BytesMut::new();
            for packet in packets {
                framed.put_u32(packet.len() as u32);
                framed.extend_from_slice(&packet);
            }
            response(StatusCode::OK, "application/octet-stream", framed.freeze())
        }
        _ => response(StatusCode::NOT_FOUND, "text/plain", Bytes::new()),
    };
    Ok(reply)
}

fn response(code: StatusCode, content_type: &'static str, bytes: Bytes) -> Response<Full<Bytes>> {
    Response::builder()
        .status(code)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Full::new(bytes))
        .expect("static HTTP headers")
}
