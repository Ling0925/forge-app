#![cfg(unix)]

use std::{env, io, process::Stdio};
use tokio::{
    process::Command,
    signal::unix::{SignalKind, signal},
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let role = args
        .next()
        .ok_or_else(|| io::Error::other("missing role"))?;
    let mode = args
        .next()
        .ok_or_else(|| io::Error::other("missing mode"))?;
    if role == "grandchild" && mode != "cooperative" {
        if unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) } == libc::SIG_ERR {
            return Err(io::Error::last_os_error());
        }
        println!("ready {role} {}", std::process::id());
        std::future::pending::<()>().await;
        return Ok(());
    }
    let mut interrupt = signal(SignalKind::interrupt())?;
    let next_role = match role.as_str() {
        "root" => Some("child"),
        "child" => Some("grandchild"),
        "grandchild" => None,
        _ => return Err(io::Error::other("unknown role")),
    };
    let mut child = match next_role {
        Some(next_role) => Some(
            Command::new(env::current_exe()?)
                .args([next_role, &mode])
                .stdin(Stdio::null())
                .spawn()?,
        ),
        None => None,
    };
    println!("ready {role} {}", std::process::id());
    interrupt.recv().await;
    println!("sigint {role}");
    if role == "root" && mode == "leader-exits" {
        return Ok(());
    }
    if let Some(child) = &mut child {
        child.wait().await?;
    }
    Ok(())
}
