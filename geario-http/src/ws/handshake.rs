//! Websockets protocol helpers
use crate::Method;
use crate::StatusCode;
use crate::header;
use crate::RequestHead;
use crate::Response;
use crate::ResponseBuilder;

use super::error::HandshakeError;

/// Verify `WebSocket` handshake request and create handshake reponse.
// /// `protocols` is a sequence of known protocols. On successful handshake,
// /// the returned response headers contain the first protocol in this list
// /// which the server also knows.
pub fn handshake(req: &RequestHead) -> Result<ResponseBuilder, HandshakeError> {
    verify_handshake(req)?;
    Ok(handshake_response(req))
}

/// Verify `WebSocket` handshake request.
// /// `protocols` is a sequence of known protocols. On successful handshake,
// /// the returned response headers contain the first protocol in this list
// /// which the server also knows.
pub fn verify_handshake(req: &RequestHead) -> Result<(), HandshakeError> {
    // WebSocket accepts only GET
    if req.method != Method::GET {
        return Err(HandshakeError::GetMethodRequired);
    }

    // Check for "UPGRADE" to websocket header
    let has_hdr = if let Some(hdr) = req.headers().get(header::UPGRADE) {
        if let Ok(s) = hdr.to_str() {
            s.to_ascii_lowercase().contains("websocket")
        } else {
            false
        }
    } else {
        false
    };
    if !has_hdr {
        return Err(HandshakeError::NoWebsocketUpgrade);
    }

    // Upgrade connection
    if !req.upgrade() {
        return Err(HandshakeError::NoConnectionUpgrade);
    }

    // check supported version
    if !req.headers().contains_key(header::SEC_WEBSOCKET_VERSION) {
        return Err(HandshakeError::NoVersionHeader);
    }
    let supported_ver = {
        if let Some(hdr) = req.headers().get(header::SEC_WEBSOCKET_VERSION) {
            hdr == "13" || hdr == "8" || hdr == "7"
        } else {
            false
        }
    };
    if !supported_ver {
        return Err(HandshakeError::UnsupportedVersion);
    }

    // check client handshake for validity
    if !req.headers().contains_key(header::SEC_WEBSOCKET_KEY) {
        return Err(HandshakeError::BadWebsocketKey);
    }
    Ok(())
}

/// Create websocket's handshake response
///
/// This function returns handshake `Response`, ready to send to peer.
///
/// # Panics
///
/// `RequestHead` must contain `SEC_WEBSOCKET_KEY` header
pub fn handshake_response(req: &RequestHead) -> ResponseBuilder {
    let key = {
        let key = req.headers().get(header::SEC_WEBSOCKET_KEY).unwrap();
        crate::ws::hash_key(key.as_ref()).unwrap_or_else(|_| String::new())
    };

    Response::build(StatusCode::SWITCHING_PROTOCOLS)
        .upgrade("websocket")
        .header(header::TRANSFER_ENCODING, "chunked")
        .header(header::SEC_WEBSOCKET_ACCEPT, key)
        .take()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ResponseError;
    use crate::request::Request;

    /// A request head with the given method and headers. These tests used to
    /// go through a `TestRequest` builder in a test server that was never
    /// ported, so none of them compiled.
    fn head(method: Method, headers: &[(header::HeaderName, &'static str)]) -> Request {
        let mut req = Request::default();
        req.head_mut().method = method;
        for (name, value) in headers {
            req.headers_mut()
                .insert(name.clone(), header::HeaderValue::from_static(value));
            // `upgrade()` reads the connection type the parser worked out,
            // not the header map, so putting the header there is not enough
            // to make the request an upgrade.
            if name == header::CONNECTION && value.eq_ignore_ascii_case("upgrade") {
                req.head_mut()
                    .set_connection_type(crate::message::ConnectionType::Upgrade);
            }
        }
        req
    }

    fn get(headers: &[(header::HeaderName, &'static str)]) -> Request {
        head(Method::GET, headers)
    }

    #[test]
    fn a_handshake_is_rejected_until_every_part_is_present() {
        // Each case adds the part the previous one was missing, so the
        // sequence walks the whole validation rather than one branch of it.
        let cases: Vec<(Request, HandshakeError)> = vec![
            (
                head(Method::POST, &[]),
                HandshakeError::GetMethodRequired,
            ),
            (get(&[]), HandshakeError::NoWebsocketUpgrade),
            (
                get(&[(header::UPGRADE, "test")]),
                HandshakeError::NoWebsocketUpgrade,
            ),
            (
                get(&[(header::UPGRADE, "websocket")]),
                HandshakeError::NoConnectionUpgrade,
            ),
            (
                get(&[
                    (header::UPGRADE, "websocket"),
                    (header::CONNECTION, "upgrade"),
                ]),
                HandshakeError::NoVersionHeader,
            ),
            (
                get(&[
                    (header::UPGRADE, "websocket"),
                    (header::CONNECTION, "upgrade"),
                    (header::SEC_WEBSOCKET_VERSION, "5"),
                ]),
                HandshakeError::UnsupportedVersion,
            ),
            (
                get(&[
                    (header::UPGRADE, "websocket"),
                    (header::CONNECTION, "upgrade"),
                    (header::SEC_WEBSOCKET_VERSION, "13"),
                ]),
                HandshakeError::BadWebsocketKey,
            ),
        ];

        for (req, expected) in cases {
            assert_eq!(expected, verify_handshake(req.head()).err().unwrap());
        }
    }

    #[test]
    fn a_complete_handshake_switches_protocols() {
        let req = get(&[
            (header::UPGRADE, "websocket"),
            (header::CONNECTION, "upgrade"),
            (header::SEC_WEBSOCKET_VERSION, "13"),
            (header::SEC_WEBSOCKET_KEY, "13"),
        ]);
        assert_eq!(
            StatusCode::SWITCHING_PROTOCOLS,
            handshake_response(req.head()).finish().status()
        );
    }

    #[test]
    fn test_wserror_http_response() {
        let resp: Response = HandshakeError::GetMethodRequired.error_response();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        for err in [
            HandshakeError::NoWebsocketUpgrade,
            HandshakeError::NoConnectionUpgrade,
            HandshakeError::NoVersionHeader,
            HandshakeError::UnsupportedVersion,
            HandshakeError::BadWebsocketKey,
        ] {
            let resp: Response = err.error_response();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        }
    }
}
