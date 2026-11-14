use crate::clock::Clock;
use std::{
    ffi::OsString,
    io,
    path::PathBuf,
    process::{ExitStatus, Stdio},
    sync::Arc,
    time::Duration,
};
use tokio::process::{Child, ChildStderr, ChildStdout, Command};

pub const CANCEL_GRACE: Duration = Duration::from_secs(2);

pub struct CommandSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
}

pub struct CommandTool {
    clock: Arc<dyn Clock>,
    #[cfg(feature = "failpoints")]
    pub failpoints: crate::failpoints::Failpoints,
}

impl CommandTool {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            #[cfg(feature = "failpoints")]
            failpoints: crate::failpoints::Failpoints::default(),
        }
    }

    pub fn spawn(&self, spec: CommandSpec) -> io::Result<RunningCommand> {
        let mut command = Command::new(spec.program);
        command
            .args(spec.args)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = spec.cwd {
            command.current_dir(cwd);
        }
        let child = command.spawn()?;
        let pgid = child.id().expect("newly spawned child has a PID") as libc::pid_t;
        crate::failpoint!(self.failpoints, crate::failpoints::TOOL_AFTER_SPAWN);
        Ok(RunningCommand {
            child,
            pgid,
            clock: self.clock.clone(),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum GroupSignal {
    Interrupt,
    Kill,
}

#[derive(Debug)]
pub struct Cancellation {
    pub status: ExitStatus,
    pub signals: Vec<GroupSignal>,
}

pub struct RunningCommand {
    child: Child,
    pgid: libc::pid_t,
    clock: Arc<dyn Clock>,
}

impl RunningCommand {
    pub fn process_group(&self) -> libc::pid_t {
        self.pgid
    }
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait().await
    }

    pub async fn cancel(&mut self) -> io::Result<Cancellation> {
        let mut signals = Vec::new();
        if signal_group(self.pgid, libc::SIGINT)? {
            signals.push(GroupSignal::Interrupt);
        }
        let mut grace = self.clock.sleep(CANCEL_GRACE);
        tokio::select! {
            status = self.child.wait() => {
                let status = status?;
                // The leader can exit while descendants still own the process group.
                if !signal_group(self.pgid, 0)? { return Ok(Cancellation { status, signals }); }
                grace.await;
            }
            () = &mut grace => {}
        }
        self.child.try_wait()?;
        if signal_group(self.pgid, libc::SIGKILL)? {
            signals.push(GroupSignal::Kill);
        }
        Ok(Cancellation {
            status: self.child.wait().await?,
            signals,
        })
    }
}

fn signal_group(pgid: libc::pid_t, signal: libc::c_int) -> io::Result<bool> {
    // A negative PID addresses the entire group, including grandchildren.
    if unsafe { libc::kill(-pgid, signal) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Err(error)
    }
}

impl crate::Tool for CommandTool {
    type Request = CommandSpec;
    type Output = RunningCommand;

    fn name(&self) -> &str {
        "command"
    }
    fn has_side_effects(&self) -> bool {
        true
    }
    fn execute(
        &self,
        request: crate::ToolRequest<CommandSpec>,
    ) -> crate::ToolFuture<'_, crate::ToolResult<RunningCommand>> {
        Box::pin(async move {
            Ok(crate::ToolResult {
                call: request.call,
                output: self.spawn(request.input)?,
            })
        })
    }
}
