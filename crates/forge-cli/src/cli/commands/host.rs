//! `forge host add|list|remove`: other machines in your fleet.

use anyhow::Result;

use crate::host_add::{add_host, AddOptions, Event, LocalApprover, SshRemote, DEFAULT_INSTALL_URL};
use crate::hosts;
use crate::HostCmd;

pub(crate) fn host_cmd(cmd: HostCmd) -> Result<()> {
    match cmd {
        HostCmd::Add {
            target,
            name,
            lan,
            port,
            no_pair,
            yes,
            ssh_args,
            installer,
        } => {
            let target = hosts::validate_target(&target)?.to_string();
            let options = AddOptions {
                target: target.clone(),
                name,
                lan,
                port,
                pair: !no_pair,
                installer,
                install_url: std::env::var("FORGE_HOST_INSTALL_URL")
                    .unwrap_or_else(|_| DEFAULT_INSTALL_URL.to_string()),
            };
            let remote = SshRemote {
                ssh: std::env::var("FORGE_SSH").unwrap_or_else(|_| "ssh".into()),
                target,
                extra: ssh_args,
            };
            let outcome = add_host(
                &options,
                &remote,
                &LocalApprover { confirm: yes },
                &mut |event| match event {
                    Event::Step(text) => println!("==> {text}"),
                    Event::Remote(line) => println!("    {line}"),
                    Event::AwaitingApproval {
                        safety_code,
                        remote_code,
                    } => {
                        println!("==> Approving the new device from this machine");
                        if let (Some(local), Some(remote)) = (safety_code, remote_code) {
                            println!("    This machine computed {local}; the new machine printed {remote}.");
                        }
                    }
                },
            )?;
            hosts::upsert(outcome.record.clone())?;
            println!(
                "{} is ready{}.",
                outcome.record.name,
                if outcome.record.paired {
                    " and paired to your fleet"
                } else {
                    ""
                }
            );
            Ok(())
        }
        HostCmd::List => {
            let list = hosts::list()?;
            if list.is_empty() {
                println!("No machines added yet. Try `forge host add user@host`.");
            }
            for host in list {
                println!(
                    "{:<24} {:<32} {}",
                    host.name,
                    host.target,
                    if host.paired {
                        "paired"
                    } else {
                        "service only"
                    }
                );
            }
            Ok(())
        }
        HostCmd::Remove { name } => {
            if hosts::remove(&name)? {
                println!("Forgot {name}. It keeps running; revoke it with `forge anywhere devices --revoke` to cut its access.");
            } else {
                anyhow::bail!("no machine named {name}; see `forge host list`");
            }
            Ok(())
        }
    }
}
