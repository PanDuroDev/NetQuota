//! One transport, three clients: desktop, CLI and the Chrome native host all
//! connect to the same pipe. Frames are the protocol's length-prefixed JSON.

use std::io::{Read, Write};

use anyhow::{anyhow, bail, Context, Result};
use netquota_protocol::{encode_frame, parse_response, take_frame, Request, Response};

/// Service endpoint. Local only: the security descriptor rejects remote clients.
pub const PIPE_NAME: &str = r"\\.\pipe\NetQuota.Service";

pub struct Client {
    file: std::fs::File,
    buffer: Vec<u8>,
}

impl Client {
    /// Connect, waiting up to `timeout_ms` for the service to publish the pipe.
    pub fn connect(timeout_ms: u32) -> Result<Self> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);
        loop {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(PIPE_NAME)
            {
                Ok(file) => {
                    return Ok(Self {
                        file,
                        buffer: Vec::new(),
                    })
                }
                Err(e) if std::time::Instant::now() < deadline => {
                    let _ = e;
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => {
                    return Err(anyhow!(
                        "cannot connect to NetQuota service at {PIPE_NAME}: {e}. Is the service installed and running?"
                    ))
                }
            }
        }
    }

    pub fn call(&mut self, request: &Request) -> Result<Response> {
        let frame = encode_frame(request).map_err(|e| anyhow!(e))?;
        self.file.write_all(&frame).context("write to service")?;
        self.file.flush().ok();
        loop {
            if let Some(body) = take_frame(&mut self.buffer).map_err(|e| anyhow!(e))? {
                return parse_response(&body).map_err(|e| anyhow!(e));
            }
            let mut chunk = [0u8; 8192];
            let read = self.file.read(&mut chunk).context("read from service")?;
            if read == 0 {
                bail!("service closed the connection");
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }

    /// Convenience for callers that only need the body, not the variant.
    pub fn expect<T>(&mut self, request: &Request, pick: impl FnOnce(Response) -> Option<T>) -> Result<T> {
        let response = self.call(request)?;
        pick(response).ok_or_else(|| anyhow!("unexpected response from service"))
    }

    pub fn read_frame(&mut self) -> Result<Vec<u8>> {
        loop {
            if let Some(body) = take_frame(&mut self.buffer).map_err(|e| anyhow!(e))? {
                return Ok(body);
            }
            let mut chunk = [0u8; 8192];
            let read = self.file.read(&mut chunk)?;
            if read == 0 {
                bail!("peer closed the connection");
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }

    pub fn write_frame(&mut self, bytes: &[u8]) -> Result<()> {
        self.file.write_all(bytes)?;
        self.file.flush().ok();
        Ok(())
    }
}

/// Blocking single-threaded accept loop. Each connection is handled on its own
/// thread so a slow desktop query cannot stall the CLI or the extension.
#[cfg(windows)]
pub fn serve<H>(first_instance: bool, handler: H) -> Result<()>
where
    H: Fn(Request) -> Response + Send + Sync + Clone + 'static,
{
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, NAMED_PIPE_MODE,
    };

    // SYSTEM and Administrators full, interactive users read/write, no network.
    const SDDL: &str = "D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";
    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const PIPE_ACCESS_FIRST_INSTANCE: u32 = 0x0008_0000;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;

    let name: Vec<u16> = PIPE_NAME.encode_utf16().chain(std::iter::once(0)).collect();
    let sddl: Vec<u16> = SDDL.encode_utf16().chain(std::iter::once(0)).collect();

    let mut first = first_instance;
    loop {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                1, // SDDL_REVISION_1
                &mut descriptor,
                None,
            )
        }
        .map_err(|e| anyhow!("bad pipe security descriptor: {e}"))?;

        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: Default::default(),
        };

        let mut mode = PIPE_ACCESS_DUPLEX;
        if first {
            mode |= PIPE_ACCESS_FIRST_INSTANCE;
        }
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(mode),
                NAMED_PIPE_MODE(PIPE_REJECT_REMOTE_CLIENTS),
                16,
                64 * 1024,
                64 * 1024,
                0,
                Some(&attributes as *const SECURITY_ATTRIBUTES),
            )
        };
        unsafe {
            let _ = LocalFree(HLOCAL(descriptor.0));
        }
        first = false;

        if handle.is_invalid() {
            return Err(anyhow!("CreateNamedPipeW failed: {}", std::io::Error::last_os_error()));
        }

        // Blocking accept; the service stops by exiting the process.
        let connected = unsafe { ConnectNamedPipe(handle, None) };
        if connected.is_err() {
            let code = std::io::Error::last_os_error();
            // ERROR_PIPE_CONNECTED (535) means the client arrived first.
            if code.raw_os_error() != Some(535) {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                if code.raw_os_error() == Some(5) {
                    // Access denied on connect: keep serving other clients.
                    continue;
                }
                tracing::warn!(%code, "pipe accept failed");
                continue;
            }
        }

        let handler = handler.clone();
        // HANDLE is a raw pointer and not Send; carry the value as isize.
        let raw = handle.0 as isize;
        std::thread::spawn(move || {
            use std::os::windows::io::FromRawHandle;
            let handle = windows::Win32::Foundation::HANDLE(raw as _);
            let file = unsafe { std::fs::File::from_raw_handle(handle.0 as _) };
            handle_connection(file, handler);
            unsafe {
                let _ = DisconnectNamedPipe(handle);
            }
        });
    }
}

#[cfg(windows)]
fn handle_connection<H>(mut file: std::fs::File, handler: H)
where
    H: Fn(Request) -> Response,
{
    use std::io::{Read, Write};
    let mut buffer = Vec::new();
    loop {
        let body = match take_frame(&mut buffer) {
            Ok(Some(body)) => body,
            Ok(None) => {
                let mut chunk = [0u8; 8192];
                match file.read(&mut chunk) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        buffer.extend_from_slice(&chunk[..n]);
                        continue;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "rejecting oversized frame");
                return;
            }
        };
        let response = match netquota_protocol::parse_request(&body) {
            Ok(request) => handler(request),
            Err(message) => Response::Error {
                code: netquota_protocol::ErrorCode::BadRequest,
                message,
            },
        };
        let frame = match encode_frame(&response) {
            Ok(frame) => frame,
            Err(e) => {
                tracing::error!(error = %e, "cannot encode response");
                return;
            }
        };
        if file.write_all(&frame).is_err() {
            return;
        }
    }
}

/// Chrome and the desktop app must not be able to open the pipe with a NULL
/// DACL; this is a compile-time reminder that the descriptor above is load
/// bearing. Kept public so tests can assert the policy text once.
pub const PIPE_SDDL: &str = "D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";

#[cfg(test)]
mod tests {
    use super::*;
    use netquota_protocol::{ClientKind, PROTOCOL_VERSION};

    #[test]
    fn sddl_is_local_and_scoped() {
        assert!(PIPE_SDDL.contains(";;;SY"));
        assert!(PIPE_SDDL.contains(";;;BA"));
        assert!(!PIPE_SDDL.contains(";;;WD"), "world must not access the pipe");
        assert!(PIPE_SDDL.starts_with("D:"), "DACL only, no SACL");
    }

    #[test]
    fn hello_frame_survives_the_transport() {
        let request = Request::Hello {
            version: PROTOCOL_VERSION,
            client: ClientKind::Cli,
            extension_id: None,
        };
        let mut bytes = encode_frame(&request).unwrap();
        let body = take_frame(&mut bytes).unwrap().unwrap();
        assert_eq!(netquota_protocol::parse_request(&body).unwrap(), request);
    }
}
