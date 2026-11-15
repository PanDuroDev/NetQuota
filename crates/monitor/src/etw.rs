//! Real-time kernel network ETW session.
//!
//! The `Microsoft-Windows-Kernel-Network` provider reports per-process byte
//! counts for TCP and UDP over IPv4 and IPv6. Retransmit events are excluded so
//! bytes are counted once, and the payload is only read for the eight-byte
//! header (PID + size), which is present on every data event.
//!
//! This session needs membership in the built-in `Performance Log Users` group
//! (or LocalSystem). Without it `StartTraceW` fails and the supervisor reports
//! per-application usage as unavailable instead of inventing numbers.

use std::sync::Mutex;

use anyhow::{anyhow, Result};
use netquota_core::quota::Direction;
use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, CONTROLTRACE_HANDLE,
    EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_RECORD, EVENT_TRACE_CONTROL_STOP,
    EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE,
    PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME, PROCESSTRACE_HANDLE,
    WNODE_FLAG_TRACED_GUID,
};

/// `Microsoft-Windows-Kernel-Network`.
const PROVIDER: GUID = GUID::from_u128(0x7dd4_2a49_5329_4832_8dfd_43d9_7915_3a88);
const SESSION_NAME: &str = "NetQuota-KernelNetwork";
const MAX_BUFFERED_EVENTS: usize = 65_536;

// Event IDs that carry payload bytes. 10/26 TCP send, 11/27 TCP receive,
// 42/58 UDP send, 43/59 UDP receive. Connect, disconnect, retransmit and
// failure events are deliberately not counted.
const TCP_V4_SEND: u16 = 10;
const TCP_V4_RECV: u16 = 11;
const TCP_V6_SEND: u16 = 26;
const TCP_V6_RECV: u16 = 27;
const UDP_V4_SEND: u16 = 42;
const UDP_V4_RECV: u16 = 43;
const UDP_V6_SEND: u16 = 58;
const UDP_V6_RECV: u16 = 59;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawEvent {
    pub pid: u32,
    pub direction: Direction,
    pub bytes: u64,
}

struct Collector {
    events: Vec<RawEvent>,
    dropped: u64,
}

static COLLECTOR: Mutex<Collector> = Mutex::new(Collector {
    events: Vec::new(),
    dropped: 0,
});

fn direction_for(id: u16) -> Option<Direction> {
    match id {
        TCP_V4_SEND | TCP_V6_SEND | UDP_V4_SEND | UDP_V6_SEND => Some(Direction::Upload),
        TCP_V4_RECV | TCP_V6_RECV | UDP_V4_RECV | UDP_V6_RECV => Some(Direction::Download),
        _ => None,
    }
}

unsafe extern "system" fn on_event(record: *mut EVENT_RECORD) {
    if record.is_null() {
        return;
    }
    let record = &*record;
    let Some(direction) = direction_for(record.EventHeader.EventDescriptor.Id) else {
        return;
    };
    let length = record.UserDataLength as usize;
    // Layout: u32 pid, u32 bytes, addresses, ports. Only the first 8 bytes are
    // guaranteed, so address fields are ignored here.
    if length < 8 || record.UserData.is_null() {
        return;
    }
    let data = std::slice::from_raw_parts(record.UserData as *const u8, length);
    let pid = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let bytes = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as u64;
    if bytes == 0 {
        return;
    }
    if let Ok(mut collector) = COLLECTOR.try_lock() {
        if collector.events.len() < MAX_BUFFERED_EVENTS {
            collector.events.push(RawEvent {
                pid,
                direction,
                bytes,
            });
        } else {
            collector.dropped += 1;
        }
    }
}

/// Take everything buffered since the last call. The second value is the number
/// of events discarded because the consumer fell behind.
pub fn drain() -> (Vec<RawEvent>, u64) {
    let mut guard = match COLLECTOR.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let events = std::mem::take(&mut guard.events);
    let dropped = std::mem::take(&mut guard.dropped);
    (events, dropped)
}

fn wide(name: &str) -> Vec<u16> {
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Props {
    buffer: Vec<u8>,
}

impl Props {
    fn new(name: &str, log_file_mode: u32) -> Self {
        let name = wide(name);
        let header = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
        let mut buffer = vec![0u8; header + name.len() * 2];
        unsafe {
            let props = buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
            (*props).Wnode.BufferSize = buffer.len() as u32;
            (*props).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            (*props).Wnode.ClientContext = 1;
            (*props).LogFileMode = log_file_mode;
            (*props).LoggerNameOffset = header as u32;
            std::ptr::copy_nonoverlapping(
                name.as_ptr() as *const u8,
                buffer.as_mut_ptr().add(header),
                name.len() * 2,
            );
        }
        Self { buffer }
    }

    fn as_mut_ptr(&mut self) -> *mut EVENT_TRACE_PROPERTIES {
        self.buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES
    }
}

pub struct EtwSession {
    handle: CONTROLTRACE_HANDLE,
    trace: PROCESSTRACE_HANDLE,
    thread: Option<std::thread::JoinHandle<()>>,
    name: Vec<u16>,
    props: Props,
}

impl EtwSession {
    pub fn start() -> Result<Self> {
        let mut props = Props::new(SESSION_NAME, EVENT_TRACE_REAL_TIME_MODE);
        let mut name = wide(SESSION_NAME);
        let mut handle = CONTROLTRACE_HANDLE::default();

        let mut status = unsafe { start_session(&mut handle, &mut name, props.as_mut_ptr()) };
        if status.0 == 183 {
            // A previous instance of this process died without stopping the
            // session. Stop the stale session, then retry once.
            let mut stale = Props::new(SESSION_NAME, 0);
            unsafe {
                let _ = ControlTraceW(
                    CONTROLTRACE_HANDLE::default(),
                    PCWSTR(name.as_ptr()),
                    stale.as_mut_ptr(),
                    EVENT_TRACE_CONTROL_STOP,
                );
            }
            status = unsafe { start_session(&mut handle, &mut name, props.as_mut_ptr()) };
        }
        if status.0 != 0 {
            return Err(anyhow!(
                "StartTraceW failed with code {} (needs administrator or Performance Log Users)",
                status.0
            ));
        }

        let enabled = unsafe {
            EnableTraceEx2(
                handle,
                &PROVIDER,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER.0 as u32,
                5,
                0,
                0,
                0,
                None,
            )
        };
        if enabled.0 != ERROR_SUCCESS_CODE {
            unsafe {
                let mut stop = Props::new(SESSION_NAME, 0);
                let _ = ControlTraceW(handle, PCWSTR::null(), stop.as_mut_ptr(), EVENT_TRACE_CONTROL_STOP);
            }
            return Err(anyhow!("EnableTraceEx2 failed with code {}", enabled.0));
        }

        let mut logfile = EVENT_TRACE_LOGFILEW::default();
        logfile.LoggerName = PWSTR(name.as_mut_ptr());
        logfile.Anonymous1.ProcessTraceMode =
            PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(on_event);

        let trace = unsafe { OpenTraceW(&mut logfile) };
        if trace.Value == 0 || trace.Value == u64::MAX {
            unsafe {
                let mut stop = Props::new(SESSION_NAME, 0);
                let _ = ControlTraceW(handle, PCWSTR::null(), stop.as_mut_ptr(), EVENT_TRACE_CONTROL_STOP);
            }
            return Err(anyhow!("OpenTraceW failed for {SESSION_NAME}"));
        }

        let thread = std::thread::Builder::new()
            .name("netquota-etw".into())
            .spawn(move || unsafe {
                // Blocks until the session is stopped.
                let _ = ProcessTrace(&[trace], None, None);
            })?;

        Ok(Self {
            handle,
            trace,
            thread: Some(thread),
            name,
            props,
        })
    }

    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        unsafe {
            let _ = ControlTraceW(
                self.handle,
                PCWSTR(self.name.as_ptr()),
                self.props.as_mut_ptr(),
                EVENT_TRACE_CONTROL_STOP,
            );
            let _ = CloseTrace(self.trace);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for EtwSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

const ERROR_SUCCESS_CODE: u32 = 0;

/// Stop a session left behind by a process that died without cleanup. Returns
/// true when a stale session was actually found and stopped.
pub fn recover_stale() -> bool {
    let name = wide(SESSION_NAME);
    let mut props = Props::new(SESSION_NAME, 0);
    unsafe {
        let status = ControlTraceW(
            CONTROLTRACE_HANDLE::default(),
            PCWSTR(name.as_ptr()),
            props.as_mut_ptr(),
            EVENT_TRACE_CONTROL_STOP,
        );
        // ERROR_WMI_INSTANCE_NOT_FOUND (4201) means there was nothing to stop.
        status.0 == 0
    }
}

unsafe fn start_session(
    handle: &mut CONTROLTRACE_HANDLE,
    name: &mut [u16],
    props: *mut EVENT_TRACE_PROPERTIES,
) -> WIN32_ERROR {
    windows::Win32::System::Diagnostics::Etw::StartTraceW(handle, PCWSTR(name.as_ptr()), props)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_data_events_count() {
        assert_eq!(direction_for(10), Some(Direction::Upload));
        assert_eq!(direction_for(27), Some(Direction::Download));
        assert_eq!(direction_for(43), Some(Direction::Download));
        assert_eq!(direction_for(58), Some(Direction::Upload));
        // Connect, retransmit and failure events are not traffic.
        for id in [12, 13, 14, 15, 17, 30, 49] {
            assert_eq!(direction_for(id), None, "event {id} must not count bytes");
        }
    }

    #[test]
    fn provider_guid_matches_kernel_network() {
        assert_eq!(PROVIDER, GUID::from_u128(0x7dd4_2a49_5329_4832_8dfd_43d9_7915_3a88));
    }
}
