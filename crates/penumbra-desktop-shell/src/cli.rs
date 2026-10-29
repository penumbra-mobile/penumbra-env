//! Command line argument parsing for the Penumbra Desktop Shell.

use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "penumbra-desktop-shell",
    about = "Native Wayland Desktop Convergence Shell (Top Panel, Dock, Launcher) built with Slint and layer-shell-slint",
    version
)]
pub struct CliArgs {
    /// Designated output connector to attach the desktop shell (e.g., "DP-1", "HDMI-A-1", or "external").
    #[arg(short, long, default_value = "external")]
    pub output: String,

    /// Run only the Top Desktop Panel.
    #[arg(long, default_value_t = false)]
    pub panel_only: bool,

    /// Run only the Bottom Desktop Dock.
    #[arg(long, default_value_t = false)]
    pub dock_only: bool,

    /// Run only the Centered Desktop Launcher.
    #[arg(long, default_value_t = false)]
    pub launcher_only: bool,

    /// Total number of desktop workspaces.
    #[arg(long, default_value_t = 4)]
    pub workspaces: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_args_parsing_defaults() {
        let args = CliArgs::parse_from(["penumbra-desktop-shell"]);
        assert_eq!(args.output, "external");
        assert!(!args.panel_only);
        assert!(!args.dock_only);
        assert!(!args.launcher_only);
        assert_eq!(args.workspaces, 4);
    }

    #[test]
    fn test_cli_args_custom_output_and_modes() {
        let args = CliArgs::parse_from([
            "penumbra-desktop-shell",
            "--output",
            "HDMI-A-1",
            "--panel-only",
            "--workspaces",
            "6",
        ]);
        assert_eq!(args.output, "HDMI-A-1");
        assert!(args.panel_only);
        assert!(!args.dock_only);
        assert_eq!(args.workspaces, 6);
    }
}
