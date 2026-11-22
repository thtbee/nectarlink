// SPDX-License-Identifier: GPL-3.0-or-later

use std::{convert::Infallible, sync::Arc, time::Duration};

use anyhow::Result;
use bytes::{BufMut, Bytes, BytesMut};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode, body::Incoming, header, server::conn::http1, service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use iroh::PublicKey;
use tokio::{net::TcpListener, task::JoinSet};

use super::{Route, Shared, transfer::WINDOW};

const PAGE: &[u8] = include_bytes!("../../anysend-web/site/index.html");
const SCRIPT: &[u8] = include_bytes!("../../anysend-web/site/receive.js");

pub(super) async fn serve(listener: TcpListener, session: Shared, site_origin: String) -> Result<()> {
    let host = listener.local_addr()?.to_string();
    let origins = Arc::new(Origins { local: format!("http://{host}"), host, site: site_origin });
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept(), if connections.len() < 512 => {
                let (stream, _) = accepted?;
                let _ = stream.set_nodelay(true);
                let session = session.clone();
                let origins = origins.clone();
                connections.spawn(async move {
                    let service = service_fn(move |request| reply(request, session.clone(), origins.clone()));
                    let _ = http1::Builder::new()
                        .timer(TokioTimer::new())
                        .header_read_timeout(Duration::from_secs(10))
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
            _ = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

struct Origins {
    host: String,
    local: String,
    site: String,
}

async fn reply(
    request: Request<Incoming>,
    session: Shared,
    origins: Arc<Origins>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    if request.headers().get(header::HOST).and_then(|value| value.to_str().ok()) != Some(&origins.host) {
        return Ok(empty(StatusCode::BAD_REQUEST));
    }

    let packets = matches!(request.uri().path(), "/packet" | "/probe");
    if packets
        && request.headers().get(header::ORIGIN).is_some_and(|origin| {
            origin.as_bytes() != origins.local.as_bytes() && origin.as_bytes() != origins.site.as_bytes()
        })
    {
        return Ok(empty(StatusCode::FORBIDDEN));
    }

    let cors = packets
        && request
            .headers()
            .get(header::ORIGIN)
            .is_some_and(|origin| origin.as_bytes() == origins.site.as_bytes());
    let mut result = match (request.method(), request.uri().path()) {
        (&Method::GET, "/") => response(StatusCode::OK, "text/html; charset=utf-8", Bytes::from_static(PAGE)),
        (&Method::GET, "/receive.js") => {
            response(StatusCode::OK, "application/javascript; charset=utf-8", Bytes::from_static(SCRIPT))
        }
        (&Method::OPTIONS, "/packet" | "/probe") if cors => empty(StatusCode::NO_CONTENT),
        (&Method::POST, "/probe") => match body(request, 32).await {
            Ok(challenge) if challenge.len() == 32 => {
                response(StatusCode::OK, "application/octet-stream", session.probe(&challenge))
            }
            Ok(_) => empty(StatusCode::BAD_REQUEST),
            Err(error) => empty(error),
        },
        (&Method::POST, "/packet") => packet_reply(request, &session).await,
        _ => empty(StatusCode::NOT_FOUND),
    };
    if packets {
        result.headers_mut().insert(header::VARY, header::HeaderValue::from_static("Origin"));
    }
    if cors {
        let headers = result.headers_mut();
        headers
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origins.site.parse().expect("valid HTTPS origin"));
        headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, header::HeaderValue::from_static("POST"));
        headers
            .insert(header::ACCESS_CONTROL_ALLOW_HEADERS, header::HeaderValue::from_static("content-type"));
        headers.insert(header::ACCESS_CONTROL_MAX_AGE, header::HeaderValue::from_static("600"));
    }
    Ok(result)
}

async fn packet_reply(request: Request<Incoming>, session: &Shared) -> Response<Full<Bytes>> {
    const MAX_PACKET_OVERHEAD: usize = 33;
    let max_packet = Route::Lan.chunk() + MAX_PACKET_OVERHEAD;
    let payload = match body(request, 32 + WINDOW * (4 + max_packet)).await {
        Ok(payload) => payload,
        Err(error) => return empty(error),
    };
    let Some((id, mut packets)) = payload.split_at_checked(32) else {
        return empty(StatusCode::BAD_REQUEST);
    };
    let Ok(id) = PublicKey::from_bytes(id.try_into().expect("32-byte key")) else {
        return empty(StatusCode::BAD_REQUEST);
    };
    let mut frames = Vec::with_capacity(WINDOW);
    while !packets.is_empty() {
        let Some(length) = super::protocol::read_index(packets).map(|length| length as usize) else {
            return empty(StatusCode::BAD_REQUEST);
        };
        if length == 0 || length > max_packet || frames.len() == WINDOW {
            return empty(StatusCode::BAD_REQUEST);
        }
        let Some((frame, rest)) = packets[4..].split_at_checked(length) else {
            return empty(StatusCode::BAD_REQUEST);
        };
        frames.push(frame);
        packets = rest;
    }
    if frames.len() > 1 && frames.iter().any(|frame| frame[0] != 3) {
        return empty(StatusCode::BAD_REQUEST);
    }
    let mut framed = BytesMut::new();
    for frame in frames {
        for packet in session.handle(Route::Lan, id, frame).await {
            framed.put_u32(packet.len() as u32);
            framed.extend_from_slice(&packet);
        }
    }
    response(StatusCode::OK, "application/octet-stream", framed.freeze())
}

async fn body(request: Request<Incoming>, limit: usize) -> Result<Bytes, StatusCode> {
    let content =
        tokio::time::timeout(Duration::from_secs(20), Limited::new(request.into_body(), limit).collect())
            .await
            .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
            .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    Ok(content.to_bytes())
}

fn empty(code: StatusCode) -> Response<Full<Bytes>> {
    response(code, "text/plain", Bytes::new())
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
