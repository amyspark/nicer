use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process;
use std::process::Command;
use std::sync::{Arc, Mutex};
use structopt::StructOpt;

#[cfg(all(unix, not(target_os = "macos")))]
fn nice_process() -> Result<()>{
    unsafe {
        use nix::libc;

        let status = libc::setpriority(libc::PRIO_PROCESS, 0, 19);

        match status {
            0 => Ok(()),
            _ => {
                let error = std::io::Error::last_os_error();

                match error.raw_os_error() {
                    Some(0) => Ok(()),
                    _ => Err(anyhow::Error::new(error))
                }
            }
        }
    }
}

#[cfg(all(unix, target_os = "macos"))]
fn nice_process() -> Result<()>{
    unsafe {
        use nix::libc;

        libc::setpriority(libc::PRIO_PROCESS, 0, libc::PRIO_DARWIN_BG);

        libc::setpriority(libc::PRIO_DARWIN_PROCESS, 0, libc::PRIO_DARWIN_BG);

        match status {
            0 => Ok(()),
            _ => {
                // Darwin returns ESRCH even though both values are correctly set; skip return
                // print!("{}\n", libc::getpriority(libc::PRIO_DARWIN_PROCESS, 0));
                // print!("{}\n", libc::getpriority(libc::PRIO_PROCESS, 0));
                // let error = std::io::Error::last_os_error();

                match error.raw_os_error() {
                    Some(0) => Ok(()),
                    _ => Err(anyhow::Error::new(error))
                }
                // return Ok(());
            }
        }
    }
}

#[cfg(windows)]
fn nice_process() -> Result<()>{
    unsafe {
        use winapi::shared::minwindef::{TRUE, FALSE};
        use winapi::um::winbase::IDLE_PRIORITY_CLASS;
        use winapi::um::processthreadsapi::{GetCurrentProcess, SetPriorityClass};

        let h_process = GetCurrentProcess();
        let status = SetPriorityClass(h_process, IDLE_PRIORITY_CLASS);

        match status {
            TRUE => Ok(()),
            FALSE => {
                let error = std::io::Error::last_os_error();

                match error.raw_os_error() {
                    Some(0) => Ok(()),
                    _ => Err(anyhow::Error::new(error))
                }
            }
        }
    }
}

#[cfg(all(unix,  target_os= "macos"))]
fn wakelock(process: &str, pid: u32) {
    unsafe {
        use core_foundation::string::{CFStringRef, CFStringCreateWithCString};
        use core_foundation::date::{CFTimeInterval};
        use nix::libc::{c_int};
        use std::ffi::CString;

        let prevent_system_sleep: CString = CString::new("PreventUserIdleSystemSleep").unwrap();
        let named: CString = CString::new("nicer").unwrap();
        let detailsd: CString = CString::new(format!("Hi from Rust! We're keeping your Mac awake on behalf of {:?} (pid {})", process, pid)).unwrap();
        // let localizedd: CString = CString::new("Hello from Rust!").unwrap();

        #[allow(non_snake_case, unused_variables)]
        let kIOPMAssertionLevelOn : u32 = 255;
        #[allow(non_snake_case, unused_variables)]
        let kIOPMAssertionLevelOff: u32 = 0;
        #[allow(non_snake_case)]
        let kCFStringEncodingASCII: u32= 0x0600;
        #[allow(non_snake_case)]
        let kIOPMAssertPreventUserIdleSystemSleep: CFStringRef = CFStringCreateWithCString(std::ptr::null(), prevent_system_sleep.as_ptr(), kCFStringEncodingASCII);
        let name: CFStringRef = CFStringCreateWithCString(std::ptr::null(), named.as_ptr(), kCFStringEncodingASCII);
        let details: CFStringRef = CFStringCreateWithCString(std::ptr::null(), detailsd.as_ptr(), kCFStringEncodingASCII);
        // let localized: CFStringRef = CFStringCreateWithCString(std::ptr::null(), localizedd.as_ptr(), kCFStringEncodingASCII);

        #[link(name = "IOKit", kind = "framework")]
        extern {
            #[allow(dead_code)]
            fn IOPMAssertionCreateWithName(AssertionType: CFStringRef, AssertionLevel: u32, AssertionName: CFStringRef, AssertionID: *mut u32) -> c_int;

            fn IOPMAssertionCreateWithDescription(AssertionType: CFStringRef,  Name: CFStringRef, Details: CFStringRef,  HumanReadableReason: CFStringRef, LocalizationBundlePath: CFStringRef, Timeout: CFTimeInterval, TimeoutAction: CFStringRef, AssertionID: *mut u32) -> c_int;
        }

        let mut id : u32 = 0;
        // IOPMAssertionCreateWithName(kIOPMAssertPreventUserIdleSystemSleep, kIOPMAssertionLevelOn, name, &mut id);
        // HumanReadableReason is ignored if non localizable
        IOPMAssertionCreateWithDescription(kIOPMAssertPreventUserIdleSystemSleep, name, details, std::ptr::null(), std::ptr::null(), 0.0, std::ptr::null(), &mut id);
    }
}

#[cfg(windows)]
fn wakelock(_process: &str, _pid: u32) {
    unsafe {
        use winapi::um::winbase::{SetThreadExecutionState};
        use winapi::um::winnt::{ES_CONTINUOUS};

        SetThreadExecutionState(ES_SYSTEM_REQUIRED | ES_CONTINUOUS);
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn wakelock(_process: &str, _pid: u32) {
    eprintln!("Linux has no caffeine, sadly.");
}

#[derive(StructOpt, Debug)]
#[structopt(about = "Automagically call your tools with background priority")]
struct Opt {
    /// Keep the system awake (supported on Windows and macOS).
    #[structopt(short, long)]
    caffeinate: bool,

    /// Name or path to the program I'll background to.
    #[structopt(parse(from_os_str))]
    program: PathBuf,

    /// Arguments to the program.
    #[structopt(parse(from_os_str))]
    args: Vec<PathBuf>,
}

fn main() -> Result<()> {
    let opt = Opt::from_args();
    nice_process()?;

    let program = opt.program.clone();

    let cmd = Command::new(opt.program).args(opt.args).spawn().context("Unable to spawn program")?;

    if opt.caffeinate {
        wakelock(&program.to_string_lossy(), cmd.id());
    }

    let arc = Arc::new(Mutex::new(cmd));

    #[cfg(unix)] {
        use nix::unistd::Pid;
        use nix::sys::signal::{kill, Signal};

        let arc_handler = arc.clone();
        ctrlc::set_handler(move || {
            let pid = Pid::from_raw(arc_handler.lock().unwrap().id() as i32);
            kill(pid, Signal::SIGINT).context("Unable to kill the program").unwrap();
        }).context("Unable to set the signal handler")?;
    }

    let status = arc.lock().unwrap().wait().context("Unable to wait for the program")?;

    match status.code() {
        Some(i) => process::exit(i),
        None => {
            #[cfg(unix)] {
                use std::os::unix::process::ExitStatusExt;
                process::exit(status.signal().unwrap_or_else(|| 9) + 128);
            }

            #[cfg(windows)]
            process::exit(127);
        }
    };
}
