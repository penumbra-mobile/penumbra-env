use std::process::Command;
use tracing::{error, info};

pub fn poweroff() {
    info!("Requesting system poweroff");
    if let Err(e) = Command::new("systemctl").arg("poweroff").spawn() {
        error!("systemctl poweroff failed: {:?}, attempting loginctl", e);
        let _ = Command::new("loginctl").arg("poweroff").spawn();
    }
}

pub fn reboot() {
    info!("Requesting system reboot");
    if let Err(e) = Command::new("systemctl").arg("reboot").spawn() {
        error!("systemctl reboot failed: {:?}, attempting loginctl", e);
        let _ = Command::new("loginctl").arg("reboot").spawn();
    }
}

pub fn suspend() {
    info!("Requesting system suspend");
    if let Err(e) = Command::new("systemctl").arg("suspend").spawn() {
        error!("systemctl suspend failed: {:?}, attempting loginctl", e);
        let _ = Command::new("loginctl").arg("suspend").spawn();
    }
}
