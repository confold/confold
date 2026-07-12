//! OS shell / file-manager integration — install, uninstall, and status check.
//!
//! Exposes Tauri commands that the frontend calls to manage context-menu entries:
//! - macOS: Quick Action `.workflow` bundles in `~/Library/Services/`. Menu items change
//!   dynamically via `regenerate_services.sh`: "Select Origin/Destination" when nothing is
//!   cached, "Change Origin" / "Compare with Origin" when one side is set, etc.
//! - Linux: a `.desktop` file with Actions in `~/.local/share/applications/`.
//! - Windows: registry verbs under `HKCU\...\Directory\shell\` (not yet implemented).
//!
//! A two-file cache (~/.cache/confold/shell-origin.txt + shell-destination.txt) tracks which
//! side is selected. When both are set, the script launches `confold://compare` automatically.

#[allow(unused_imports)]
use tauri::State;
#[allow(unused_imports)]
use std::path::PathBuf;

use crate::scan::AppState;

#[derive(serde::Serialize)]
pub(crate) struct ShellIntegrationStatus {
    pub(crate) installed: bool,
}

const CACHE_DIR_NAME: &str = "confold";

fn cache_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into())).join(CACHE_DIR_NAME)
    } else {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".cache").join(CACHE_DIR_NAME)
    }
}

// ── macOS: Quick Action .workflow generation ────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
const SERVICES_DIR: &str = "Library/Services";

#[cfg(target_os = "macos")]
fn services_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(SERVICES_DIR)
}

#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Removes all `Confold *.workflow` bundles, copies templates with state-appropriate names,
/// patches the menu title via `plutil`, and flushes `pbs`. Called by the shell scripts after
/// every cache change so the Finder Services menu reflects the current selection state.
#[cfg(target_os = "macos")]
const REGENERATE_SH: &str = r#"#!/bin/sh
DIR="$HOME/.cache/confold"
SRV="$HOME/Library/Services"
TPL="$DIR/workflows"
[ -d "$TPL/origin.workflow" ] || exit 0
rm -rf "$SRV"/Confold*.workflow
if [ -f "$DIR/shell-origin.txt" ]; then
  O_NAME="Confold Change Origin"
  D_NAME="Confold Compare"
elif [ -f "$DIR/shell-destination.txt" ]; then
  O_NAME="Confold Compare"
  D_NAME="Confold Change Destination"
else
  O_NAME="Confold Select Origin"
  D_NAME="Confold Select Destination"
fi
cp -R "$TPL/origin.workflow" "$SRV/$O_NAME.workflow"
cp -R "$TPL/destination.workflow" "$SRV/$D_NAME.workflow"
plutil -replace NSServices.0.NSMenuItem.default -string "$O_NAME" "$SRV/$O_NAME.workflow/Contents/Info.plist" >/dev/null 2>&1
plutil -replace CFBundleName -string "$O_NAME" "$SRV/$O_NAME.workflow/Contents/Info.plist" >/dev/null 2>&1
plutil -replace NSServices.0.NSMenuItem.default -string "$D_NAME" "$SRV/$D_NAME.workflow/Contents/Info.plist" >/dev/null 2>&1
plutil -replace CFBundleName -string "$D_NAME" "$SRV/$D_NAME.workflow/Contents/Info.plist" >/dev/null 2>&1
/System/Library/CoreServices/pbs -update >/dev/null 2>&1
"#;

#[cfg(target_os = "macos")]
fn remove_all_confold_workflows() {
    if let Ok(entries) = std::fs::read_dir(services_path()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("Confold") && name.ends_with(".workflow") {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// Caches the selected folder; when the other side is already cached, launches `confold://compare`.
/// On macOS, calls `regenerate_services.sh` afterwards so the Finder menu updates dynamically.
/// `side` must be "origin" or "destination".
fn build_side_script(side: &str) -> String {
    let (this_file, other_file, label) = if side == "origin" {
        ("shell-origin.txt", "shell-destination.txt", "Origin")
    } else {
        ("shell-destination.txt", "shell-origin.txt", "Destination")
    };
    let enc_self = if side == "origin" { "\"$f\"" } else { "\"$other\"" };
    let enc_other = if side == "origin" { "\"$other\"" } else { "\"$f\"" };

    #[cfg(target_os = "macos")]
    let (open_cmd, notify_cmd, regenerate) = (
        "/usr/bin/open",
        format!(r#"/usr/bin/osascript -e 'display notification "{label} selected: '"$f"'" with title "Confold"'"#),
        r#"  sh "$dir/regenerate_services.sh" 2>/dev/null
"#,
    );
    #[cfg(not(target_os = "macos"))]
    let (open_cmd, notify_cmd, regenerate) = (
        "xdg-open",
        format!(r#"notify-send -a Confold "{label} selected: $f" 2>/dev/null || true"#),
        "",
    );

    format!(
        r#"for f in "$@"; do
  dir="$HOME/.cache/confold"
  mkdir -p "$dir"
  printf '%s' "$f" > "$dir/{this_file}"
  other=$(cat "$dir/{other_file}" 2>/dev/null || true)
  if [ -n "$other" ]; then
    enc_o=$(/usr/bin/python3 -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1], safe=''))" {enc_self})
    enc_d=$(/usr/bin/python3 -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1], safe=''))" {enc_other})
    rm -f "$dir/shell-origin.txt" "$dir/shell-destination.txt"
    {open} "confold://compare?origin=${{enc_o}}&destination=${{enc_d}}"
  else
    {notify}
  fi
{regenerate}  break
done"#,
        this_file = this_file,
        other_file = other_file,
        enc_self = enc_self,
        enc_other = enc_other,
        open = open_cmd,
        notify = notify_cmd,
        regenerate = regenerate,
    )
}

/// Declares one `NSService` accepting folders/files, visible in Finder's Services menu.
#[cfg(target_os = "macos")]
fn workflow_info_plist(menu_name: &str, bundle_suffix: &str) -> String {
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>{menu}</string>
	<key>CFBundleIdentifier</key>
	<string>com.confold.services.{suffix}</string>
	<key>CFBundleShortVersionString</key>
	<string>1.0</string>
	<key>NSServices</key>
	<array>
		<dict>
			<key>NSMenuItem</key>
			<dict>
				<key>default</key>
				<string>{menu}</string>
			</dict>
			<key>NSMessage</key>
			<string>runWorkflowAsService</string>
			<key>NSSendFileTypes</key>
			<array>
				<string>public.folder</string>
				<string>public.item</string>
			</array>
		</dict>
	</array>
</dict>
</plist>"##,
        menu = xml_escape(menu_name),
        suffix = bundle_suffix,
    )
}

/// Automator workflow with a Run Shell Script action.
/// CRITICAL: the script body key is `COMMAND_STRING` (all caps) — `CommandString` silently fails.
#[cfg(target_os = "macos")]
fn workflow_document_wflow(script_body: &str) -> String {
    let escaped = xml_escape(script_body);
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<true/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.path</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>2.0.3</string>
				<key>AMApplication</key>
				<array>
					<string>Automator</string>
				</array>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.path</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>{script}</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>1</integer>
					<key>shell</key>
					<string>/bin/sh</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>CFBundleVersion</key>
				<string>2.0.3</string>
				<key>CanShowSelectedItemsWhenRun</key>
				<false/>
				<key>CanShowWhenRun</key>
				<true/>
				<key>Category</key>
				<array>
					<string>AMCategoryUtilities</string>
				</array>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>A0000000-0000-0000-0000-000000000001</string>
				<key>Keywords</key>
				<array>
					<string>Shell</string>
					<string>Script</string>
					<string>Command</string>
					<string>Run</string>
				</array>
				<key>OutputUUID</key>
				<string>B0000000-0000-0000-0000-000000000001</string>
				<key>UUID</key>
				<string>C0000000-0000-0000-0000-000000000001</string>
				<key>UnlocalizedApplications</key>
				<array>
					<string>Automator</string>
				</array>
				<key>arguments</key>
				<dict>
					<key>0</key>
					<dict>
						<key>default value</key>
						<integer>0</integer>
						<key>name</key>
						<string>inputMethod</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>0</string>
					</dict>
					<key>1</key>
					<dict>
						<key>default value</key>
						<false/>
						<key>name</key>
						<string>CheckedForUserDefaultShell</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>1</string>
					</dict>
					<key>2</key>
					<dict>
						<key>default value</key>
						<string></string>
						<key>name</key>
						<string>source</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>2</string>
					</dict>
					<key>3</key>
					<dict>
						<key>default value</key>
						<string></string>
						<key>name</key>
						<string>COMMAND_STRING</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>3</string>
					</dict>
					<key>4</key>
					<dict>
						<key>default value</key>
						<string>/bin/sh</string>
						<key>name</key>
						<string>shell</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>4</string>
					</dict>
				</dict>
				<key>nibPath</key>
				<string>/System/Library/Automator/Run Shell Script.action/Contents/Resources/Base.lproj/main.nib</string>
			</dict>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>serviceApplicationBundleID</key>
		<string>com.apple.finder</string>
		<key>serviceInputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject.folder</string>
		<key>serviceOutputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>serviceProcessesInput</key>
		<integer>0</integer>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>"##,
        script = escaped,
    )
}

#[cfg(target_os = "macos")]
fn write_templates() -> Result<(), String> {
    let tpl = cache_dir().join("workflows");
    let origin_script = build_side_script("origin");
    let dest_script = build_side_script("destination");

    let o_contents = tpl.join("origin.workflow/Contents");
    std::fs::create_dir_all(&o_contents).map_err(|e| e.to_string())?;
    std::fs::write(o_contents.join("Info.plist"), workflow_info_plist("Confold Select Origin", "origin"))
        .map_err(|e| e.to_string())?;
    std::fs::write(o_contents.join("document.wflow"), workflow_document_wflow(&origin_script))
        .map_err(|e| e.to_string())?;

    let d_contents = tpl.join("destination.workflow/Contents");
    std::fs::create_dir_all(&d_contents).map_err(|e| e.to_string())?;
    std::fs::write(d_contents.join("Info.plist"), workflow_info_plist("Confold Select Destination", "destination"))
        .map_err(|e| e.to_string())?;
    std::fs::write(d_contents.join("document.wflow"), workflow_document_wflow(&dest_script))
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn refresh_services() {
    let _ = std::process::Command::new("/System/Library/CoreServices/pbs")
        .arg("-update")
        .output();
}

// ── Tauri commands ──────────────────────────────────────────────────────────────────────────────

#[tauri::command]
pub(crate) fn install_shell_integration(_state: State<AppState>) -> Result<(), String> {
    std::fs::create_dir_all(cache_dir()).map_err(|e| e.to_string())?;

    #[cfg(target_os = "macos")]
    {
        write_templates()?;
        std::fs::write(cache_dir().join("regenerate_services.sh"), REGENERATE_SH)
            .map_err(|e| e.to_string())?;
        let _ = std::process::Command::new("/bin/sh")
            .arg(cache_dir().join("regenerate_services.sh"))
            .output();
    }

    #[cfg(target_os = "linux")]
    {
        let origin_script = build_side_script("origin");
        let dest_script = build_side_script("destination");
        std::fs::write(cache_dir().join("set-origin.sh"), &origin_script).map_err(|e| e.to_string())?;
        std::fs::write(cache_dir().join("set-destination.sh"), &dest_script).map_err(|e| e.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(cache_dir().join("set-origin.sh"), std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        std::fs::set_permissions(cache_dir().join("set-destination.sh"), std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;

        let home = std::env::var("HOME").unwrap_or_default();
        let app_file = format!("{}/.local/share/applications/confold-compare.desktop", home);
        std::fs::create_dir_all(format!("{}/.local/share/applications", home)).map_err(|e| e.to_string())?;
        let desktop = format!(
            "[Desktop Entry]\nType=Application\nName=Confold Compare\nIcon=folder-compare\nExec=xdg-open %U\nTerminal=false\nMimeType=x-scheme-handler/confold;\nNoDisplay=true\n\n[Desktop Action SetOrigin]\nName=Confold Select Origin\nExec=sh {cache}/set-origin.sh %f\n\n[Desktop Action SetDestination]\nName=Confold Select Destination\nExec=sh {cache}/set-destination.sh %f\n",
            cache = cache_dir().display()
        );
        std::fs::write(&app_file, desktop).map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub(crate) fn uninstall_shell_integration(_state: State<AppState>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        remove_all_confold_workflows();
        let _ = std::fs::remove_dir_all(cache_dir().join("workflows"));
        let _ = std::fs::remove_file(cache_dir().join("regenerate_services.sh"));
        refresh_services();
    }

    #[cfg(target_os = "linux")]
    {
        let p = format!(
            "{}/.local/share/applications/confold-compare.desktop",
            std::env::var("HOME").unwrap_or_default()
        );
        let _ = std::fs::remove_file(&p);
    }

    let _ = std::fs::remove_file(cache_dir().join("shell-origin.txt"));
    let _ = std::fs::remove_file(cache_dir().join("shell-destination.txt"));
    let _ = std::fs::remove_file(cache_dir().join("set-origin.sh"));
    let _ = std::fs::remove_file(cache_dir().join("set-destination.sh"));

    Ok(())
}

#[tauri::command]
pub(crate) fn shell_integration_status(_state: State<AppState>) -> ShellIntegrationStatus {
    #[cfg(target_os = "macos")]
    {
        let installed = cache_dir()
            .join("workflows/origin.workflow/Contents/document.wflow")
            .exists();
        ShellIntegrationStatus { installed }
    }

    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let installed = PathBuf::from(&home)
            .join(".local/share/applications/confold-compare.desktop")
            .exists();
        ShellIntegrationStatus { installed }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        ShellIntegrationStatus { installed: false }
    }
}
