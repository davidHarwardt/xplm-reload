
set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

install-win:
    cargo build
    New-Item -ItemType Directory -Force -ErrorAction SilentlyContinue -Path "$Env:XPLANE_PLUGIN_PATH/xplm_reload/64"
    copy target/debug/xplm_reload.dll $Env:XPLANE_PLUGIN_PATH/xplm_reload/64/win.xpl


