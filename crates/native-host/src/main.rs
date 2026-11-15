//! Chrome native messaging host.
//!
//! Chrome launches this process with the calling extension's origin as the
//! first argument, e.g. `chrome-extension://abcdef.../`. The origin is checked
//! against an allow-list file before the service is contacted, so an unrelated
//! extension that somehow knows the host name still cannot speak to the pipe.
//! Chrome's 4-byte framing is byte-for-byte the same as the pipe framing.

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{bail, Result};
use netquota_ipc::Client;
use netquota_protocol::{encode_frame, parse_request, take_frame, ClientKind, ErrorCode, Request, Response, PROTOCOL_VERSION};

/// Chrome rejects host messages larger than 1 MB.
const MAX_NATIVE_MESSAGE: usize = 1024 * 1024;

fn main() {
    if let Err(e) = run() {
        // Chrome shows the message from an error response far better than it
        // shows stderr, but stderr still lands in the browser log.
        eprintln!("netquota-native-host: {e}");
        let _ = write_message(&Response::Error {
            code: ErrorCode::Internal,
            message: e.to_string(),
        });
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let origin = args.get(1).cloned().unwrap_or_default();
    let extension_id = extension_id_from_origin(&origin)
        .ok_or_else(|| anyhow::anyhow!("missing or malformed extension origin: {origin:?}"))?;

    let allow_list = read_allow_list()?;
    if !allow_list.iter().any(|id| id == &extension_id) {
        bail!("extension {extension_id} is not allow-listed for this host");
    }

    let mut client = Client::connect(500)?;
    match client.call(&Request::Hello {
        version: PROTOCOL_VERSION,
        client: ClientKind::ChromeExtension,
        extension_id: Some(extension_id),
    })? {
        Response::Hello { .. } => {}
        Response::Error { message, .. } => bail!("service refused the extension: {message}"),
        other => bail!("unexpected handshake response: {other:?}"),
    }

    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let body = match next_message(&mut input, &mut buffer)? {
            Some(body) => body,
            None => return Ok(()),
        };
        let response = match parse_request(&body) {
            Ok(Request::Hello { .. }) => Response::Pong,
            Ok(request) => client.call(&request).unwrap_or_else(|e| Response::Error {
                code: ErrorCode::Internal,
                message: format!("service unavailable: {e}"),
            }),
            Err(message) => Response::Error {
                code: ErrorCode::BadRequest,
                message,
            },
        };
        write_message(&response)?;
    }
}

fn next_message(input: &mut impl Read, buffer: &mut Vec<u8>) -> Result<Option<Vec<u8>>> {
    loop {
        // Reject the announced length before waiting for a body that will never
        // be accepted.
        if buffer.len() >= 4 {
            let announced = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
            if announced > MAX_NATIVE_MESSAGE {
                bail!("message exceeds the native messaging limit");
            }
        }
        if let Some(body) = take_frame(buffer).map_err(|e| anyhow::anyhow!(e))? {
            if body.len() > MAX_NATIVE_MESSAGE {
                bail!("message exceeds the native messaging limit");
            }
            return Ok(Some(body));
        }
        let mut chunk = [0u8; 8192];
        let read = input.read(&mut chunk)?;
        if read == 0 {
            return Ok(None);
        }
        if buffer.len() + read > MAX_NATIVE_MESSAGE + 4 {
            bail!("message exceeds the native messaging limit");
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

fn write_message(response: &Response) -> Result<()> {
    let frame = encode_frame(response).map_err(|e| anyhow::anyhow!(e))?;
    if frame.len() > MAX_NATIVE_MESSAGE {
        let fallback = encode_frame(&Response::Error {
            code: ErrorCode::Internal,
            message: "response too large for native messaging".into(),
        })
        .map_err(|e| anyhow::anyhow!(e))?;
        std::io::stdout().write_all(&fallback)?;
        std::io::stdout().flush()?;
        return Ok(());
    }
    std::io::stdout().write_all(&frame)?;
    std::io::stdout().flush()?;
    Ok(())
}

fn extension_id_from_origin(origin: &str) -> Option<String> {
    let rest = origin.strip_prefix("chrome-extension://")?;
    let id = rest.strip_suffix('/').unwrap_or(rest);
    let valid = id.len() == 32
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || ('a'..='p').contains(&c));
    valid.then(|| id.to_string())
}

fn allow_list_path() -> PathBuf {
    std::env::var("NETQUOTA_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\ProgramData\NetQuota"))
        .join("extension-id.txt")
}

fn read_allow_list() -> Result<Vec<String>> {
    let path = allow_list_path();
    let text = std::fs::read_to_string(&path).map_err(|e| {
        anyhow::anyhow!(
            "cannot read the extension allow-list at {}: {e}. Re-run the NetQuota installer.",
            path.display()
        )
    })?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_parsing_is_strict() {
        let id = "abcdefghijklmnopabcdefghijklmnop";
        assert_eq!(
            extension_id_from_origin(&format!("chrome-extension://{id}/")).as_deref(),
            Some(id)
        );
        assert!(extension_id_from_origin("https://evil.example/").is_none());
        assert!(extension_id_from_origin("chrome-extension://short/").is_none());
        assert!(extension_id_from_origin("chrome-extension://ABCDEFGHIJKLMNOPABCDEFGHIJKLMNOP/").is_none());
        assert!(extension_id_from_origin("").is_none());
    }

    #[test]
    fn framing_rejects_an_oversized_announcement() {
        let mut buffer = ((MAX_NATIVE_MESSAGE as u32) + 1).to_le_bytes().to_vec();
        assert!(next_message(&mut std::io::empty(), &mut buffer).is_err());
    }

    #[test]
    fn eof_is_a_clean_exit() {
        let mut buffer = Vec::new();
        assert!(next_message(&mut std::io::empty(), &mut buffer).unwrap().is_none());
    }
}
