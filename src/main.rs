use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process;
use std::process::Command;
use std::sync::{Arc, Mutex};
use structopt::StructOpt;

fn nice_process() -> Result<()>{
    #[cfg(unix)]
    unsafe {
        use nix::libc;

        #[cfg(all(unix, not(target_os = "macos")))]
        libc::setpriority(libc::PRIO_PROCESS, 0, 19);

        #[cfg(all(unix, target_os = "macos"))]
        libc::setpriority(libc::PRIO_PROCESS, 0, libc::PRIO_DARWIN_BG);
    }

    #[cfg(windows)]
    unsafe {
        use winapi::um::winbase::IDLE_PRIORITY_CLASS;
        use winapi::um::processthreadsapi::{GetCurrentProcess, SetPriorityClass};

        let h_process = GetCurrentProcess();
        SetPriorityClass(h_process, IDLE_PRIORITY_CLASS);
    }

    let error = std::io::Error::last_os_error();

    match error.raw_os_error() {
        Some(0) => Ok(()),
        _ => Err(anyhow::Error::new(error))
    }
}

#[derive(StructOpt, Debug)]
#[structopt(about = "Automagically call your tools with background priority")]
struct Opt {
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

    let cmd = Command::new(opt.program).args(opt.args).spawn().context("Unable to spawn program")?;
    
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
