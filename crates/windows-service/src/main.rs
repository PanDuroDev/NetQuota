//! Windows service host. Console mode (`netquota-service --console`) is the
//! development path: same code, no SCM, Ctrl+C to stop.

#![cfg_attr(not(windows), allow(dead_code))]

mod runtime;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use netquota_protocol::{ErrorCode, Request, Response};
use runtime::Runtime;

const SERVICE_NAME: &str = "NetQuota";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--install") {
        return install();
    }
    if args.iter().any(|a| a == "--uninstall") {
        return uninstall();
    }
    let console = args.iter().any(|a| a == "--console");
    let dry_run = std::env::var("NETQUOTA_DRY_RUN").is_ok_and(|v| v == "1");
    if dry_run {
        eprintln!("dry-run: firewall rules are simulated, no traffic is affected");
    }
    if console {
        let stop = Arc::new(AtomicBool::new(false));
        install_console_handler(stop.clone());
        run(stop, dry_run)
    } else {
        windows_service::service_dispatcher::start(SERVICE_NAME, ffi_service_main)
            .context("start service dispatcher")
    }
}

fn run(stop: Arc<AtomicBool>, dry_run: bool) -> Result<()> {
    init_logging();
    let mut runtime = Runtime::open(&runtime::data_dir(), dry_run)?;
    tracing::info!(version = runtime::SERVICE_VERSION, "service starting");

    // The pipe thread only forwards requests; all state stays on this thread so
    // the store, firewall COM objects and monitor are never touched concurrently.
    let (tx, rx) = mpsc::channel::<(Request, mpsc::Sender<Response>)>();
    let replies = ReplyChannel(Arc::new(Mutex::new(tx)));
    std::thread::spawn(move || {
        if let Err(e) = netquota_ipc::serve(true, move |request| {
            let (reply_tx, reply_rx) = mpsc::channel();
            let queued = match replies.0.lock() {
                Ok(guard) => guard.send((request, reply_tx)).is_ok(),
                Err(_) => false,
            };
            if !queued {
                return Response::Error {
                    code: ErrorCode::Internal,
                    message: "service is shutting down".into(),
                };
            }
            reply_rx.recv_timeout(Duration::from_secs(10)).unwrap_or(Response::Error {
                code: ErrorCode::Internal,
                message: "service did not answer in time".into(),
            })
        }) {
            tracing::error!(error = %e, "pipe server stopped");
        }
    });

    let mut last_poll = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        while let Ok((request, reply)) = rx.try_recv() {
            let response = runtime.handle(request);
            let _ = reply.send(response);
        }
        if last_poll.elapsed() >= Duration::from_millis(1_000) {
            runtime.poll();
            last_poll = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    runtime.shutdown();
    Ok(())
}

type QueuedRequest = (Request, mpsc::Sender<Response>);

#[derive(Clone)]
struct ReplyChannel(Arc<Mutex<mpsc::Sender<QueuedRequest>>>);

fn init_logging() {
    struct LogWriter(Option<std::fs::File>);

    impl std::io::Write for LogWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            match &mut self.0 {
                Some(file) => file.write(buf),
                None => Ok(buf.len()),
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            match &mut self.0 {
                Some(file) => file.flush(),
                None => Ok(()),
            }
        }
    }

    let dir = runtime::data_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("service.log");
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || {
            LogWriter(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .ok(),
            )
        })
        .with_max_level(tracing::Level::INFO)
        .try_init();
}

/// Register this executable as a LocalSystem service and start it. The desktop
/// application runs this through a UAC prompt; no other step needs admin.
fn install() -> Result<()> {
    use std::ffi::OsString;
    use windows_service::service::{
        ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState,
        ServiceType,
    };
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let source = std::env::current_exe().context("locate service executable")?;
    // The service must survive the folder it was downloaded to, so install a
    // copy under ProgramData. A running service keeps its old image, so an
    // upgrade can overwrite the file in place.
    let bin_dir = runtime::data_dir().join("bin");
    std::fs::create_dir_all(&bin_dir).context("create service directory")?;
    let binary = bin_dir.join("netquota-service.exe");
    if source != binary {
        match std::fs::copy(&source, &binary) {
            Ok(_) => {}
            // Locked by the running service: keep using the installed copy.
            Err(_) if binary.is_file() => {}
            Err(err) => return Err(err).context("copy service executable"),
        }
    }
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("NetQuota network manager"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: binary,
        launch_arguments: Vec::new(),
        dependencies: Vec::new(),
        account_name: None,
        account_password: None,
    };
    let access = ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::QUERY_STATUS;
    let service = match manager.create_service(&info, access) {
        Ok(service) => service,
        // Already installed: reuse it so upgrading the files is enough.
        Err(create_error) => manager
            .open_service(SERVICE_NAME, access)
            .map_err(|_| create_error)
            .context("create service")?,
    };
    match service.start::<&str>(&[]) {
        Ok(_) => println!("service '{SERVICE_NAME}' installed and started"),
        Err(start_error) => {
            let running = service
                .query_status()
                .map(|status| status.current_state == ServiceState::Running)
                .unwrap_or(false);
            if !running {
                return Err(start_error).context("start service");
            }
            println!("service '{SERVICE_NAME}' is already running");
        }
    }
    Ok(())
}

/// Restore NetQuota's own firewall rules, then remove the service.
fn uninstall() -> Result<()> {
    use windows_service::service::ServiceAccess;
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    if let Ok(mut firewall) =
        netquota_enforcement::Firewall::connect(runtime::data_dir().join("firewall-manifest.json"), false)
    {
        let _ = firewall.restore_all();
    }
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::STOP | ServiceAccess::DELETE)
        .context("open service")?;
    let _ = service.stop();
    service.delete().context("delete service")?;
    // Remove the installed copy; failure here is harmless for a later reinstall.
    let _ = std::fs::remove_dir_all(runtime::data_dir().join("bin"));
    println!("service '{SERVICE_NAME}' removed");
    Ok(())
}

fn install_console_handler(stop: Arc<AtomicBool>) {
    static STOP: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);
    if let Ok(mut slot) = STOP.lock() {
        *slot = Some(stop);
    }
    unsafe {
        use windows::Win32::Foundation::{BOOL, TRUE};
        use windows::Win32::System::Console::SetConsoleCtrlHandler;
        unsafe extern "system" fn handler(_: u32) -> BOOL {
            if let Ok(slot) = STOP.lock() {
                if let Some(flag) = slot.as_ref() {
                    flag.store(true, Ordering::Relaxed);
                }
            }
            TRUE
        }
        let _ = SetConsoleCtrlHandler(Some(handler), true);
    }
}

#[cfg(windows)]
fn service_main(_args: Vec<std::ffi::OsString>) {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let Ok(status_handle) = windows_service::service_control_handler::register(
        SERVICE_NAME,
        move |control| {
            use windows_service::service::ServiceControl;
            use windows_service::service_control_handler::ServiceControlHandlerResult;
            match control {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    flag.store(true, Ordering::Relaxed);
                    ServiceControlHandlerResult::NoError
                }
                _ => ServiceControlHandlerResult::NotImplemented,
            }
        },
    ) else {
        return;
    };

    use windows_service::service::{ServiceControlAccept, ServiceState, ServiceType};
    let running = windows_service::service::ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: windows_service::service::ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };
    let _ = status_handle.set_service_status(running);

    if let Err(e) = run(stop, false) {
        tracing::error!(error = %e, "service stopped with an error");
    }

    let stopped = windows_service::service::ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: windows_service::service::ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };
    let _ = status_handle.set_service_status(stopped);
}

#[cfg(windows)]
windows_service::define_windows_service!(ffi_service_main, service_main);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_channel_is_cloneable_across_threads() {
        let (tx, rx) = mpsc::channel::<(Request, mpsc::Sender<Response>)>();
        let channel = ReplyChannel(Arc::new(Mutex::new(tx)));
        let clone = channel.clone();
        let handle = std::thread::spawn(move || {
            let (reply_tx, _reply_rx) = mpsc::channel();
            clone.0.lock().unwrap().send((Request::Ping, reply_tx)).ok()
        });
        assert!(handle.join().unwrap().is_some());
        assert!(rx.recv().is_ok());
    }
}
