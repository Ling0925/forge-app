#![cfg(target_os = "linux")]

use forge_tools::{
    Tool, ToolError, ToolRequest,
    clock::FakeClock,
    command::{CANCEL_GRACE, CommandSpec, CommandTool, GroupSignal},
};
use std::{io, os::unix::process::ExitStatusExt, sync::Arc};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

#[tokio::test]
async fn cancels_complete_process_groups_using_fake_time() {
    // Adopt killed descendants so this test does not depend on the container's PID 1 reaper.
    assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }, 0);
    for mode in ["cooperative", "ignore", "leader-exits"] {
        let clock = Arc::new(FakeClock::default());
        let tool = CommandTool::new(clock.clone());
        let mut process = tool
            .spawn(CommandSpec {
                program: env!("CARGO_BIN_EXE_fake-command").into(),
                args: vec!["root".into(), mode.into()],
                cwd: None,
            })
            .unwrap();
        let pgid = process.process_group();
        let mut lines = BufReader::new(process.take_stdout().unwrap()).lines();
        let mut grandchild = None;
        for _ in 0..3 {
            let line = lines.next_line().await.unwrap().unwrap();
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields[0], "ready");
            let pid: libc::pid_t = fields[2].parse().unwrap();
            assert_eq!(unsafe { libc::getpgid(pid) }, pgid);
            if fields[1] == "grandchild" {
                grandchild = Some(pid);
            }
        }
        let task = tokio::spawn(async move { process.cancel().await.unwrap() });
        if mode == "cooperative" {
            let result = task.await.unwrap();
            assert_eq!(result.signals, [GroupSignal::Interrupt]);
            assert!(result.status.success());
            assert_eq!(
                forge_tools::clock::Clock::now(clock.as_ref()),
                std::time::Duration::ZERO
            );
        } else {
            let mut notified = Vec::new();
            while notified.len() < 2 {
                notified.push(lines.next_line().await.unwrap().unwrap());
            }
            assert!(notified.contains(&"sigint root".to_owned()));
            assert!(notified.contains(&"sigint child".to_owned()));
            clock.advance(CANCEL_GRACE / 2);
            tokio::task::yield_now().await;
            assert!(!task.is_finished());
            let stat =
                std::fs::read_to_string(format!("/proc/{}/stat", grandchild.unwrap())).unwrap();
            assert_ne!(
                stat.rsplit_once(") ").unwrap().1.split_whitespace().next(),
                Some("Z")
            );
            clock.advance(CANCEL_GRACE / 2);
            let result = task.await.unwrap();
            assert_eq!(result.signals, [GroupSignal::Interrupt, GroupSignal::Kill]);
            if mode == "ignore" {
                assert_eq!(result.status.signal(), Some(libc::SIGKILL));
            }
            let mut grandchild_killed = false;
            loop {
                let mut status = 0;
                let pid = unsafe { libc::waitpid(-pgid, &mut status, 0) };
                if pid == -1 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() == Some(libc::EINTR) {
                        continue;
                    }
                    assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
                    break;
                }
                if Some(pid) == grandchild {
                    assert!(libc::WIFSIGNALED(status));
                    assert_eq!(libc::WTERMSIG(status), libc::SIGKILL);
                    grandchild_killed = true;
                }
            }
            assert!(grandchild_killed);
        }
        assert_eq!(unsafe { libc::kill(-pgid, 0) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }
}

#[tokio::test]
async fn argv_is_passed_literally_without_a_shell() {
    let tool = CommandTool::new(Arc::new(FakeClock::default()));
    let literal = "$(touch unwanted); * && echo expanded";
    let process = tool
        .execute(ToolRequest {
            call: "call".into(),
            input: CommandSpec {
                program: "/usr/bin/printf".into(),
                args: vec!["%s".into(), literal.into()],
                cwd: None,
            },
        })
        .await
        .unwrap();
    assert_eq!(process.call, "call".into());
    let mut process = process.output;
    let mut output = String::new();
    process
        .take_stdout()
        .unwrap()
        .read_to_string(&mut output)
        .await
        .unwrap();
    assert!(process.wait().await.unwrap().success());
    assert_eq!(output, literal);
}

#[tokio::test]
async fn spawn_failure_remains_a_local_io_error() {
    let tool = CommandTool::new(Arc::new(FakeClock::default()));
    let result = tool
        .execute(ToolRequest {
            call: "call".into(),
            input: CommandSpec {
                program: "/nonexistent-forge-test-command".into(),
                args: vec![],
                cwd: None,
            },
        })
        .await;
    assert!(matches!(result, Err(ToolError::Io(error)) if error.kind() == io::ErrorKind::NotFound));
}
