#!/bin/sh
# /usr/local/libexec/wsm-gui {start|stop|logout|status|prewarm}
# The GUI chain is installed but kept disabled; this enables it on demand.
#   prewarm: WindowServer + the user's desktop session, at boot, while the host window is hidden.
#            Measured (round 3, RTX 4050): the first session after boot presents 0 frames for >30 s
#            (reims' first-use GPU memory imports), a later session ~5 s; a warm session 47-61 fps.
#            So the session is created early and kept: the first visible `wsm gui start` is warm.
#   start:   same as prewarm when needed (wsmd then shows the window)
#   stop:    nothing to do in the guest: wsmd hides the window and the session stays warm
#   logout:  end the desktop session (the next start pays the first-session cost again)
set -u
WS=system/com.apple.WindowServer
USER_NAME=$(cat /var/db/wsm/user 2>/dev/null || echo dev)
# The paravirtual GPU's driver can attach after sshd is up (seen: `wsm gui start` right after a cold
# boot found no device), so wait for it before concluding there is none.
have_gpu() {
    i=0
    while [ $i -lt 60 ]; do
        ioreg -r -c AppleParavirtGPUControl 2>/dev/null | grep -q AppleParavirtGPU && return 0
        sleep 0.5; i=$((i + 1))
    done
    return 1
}

# `launchctl print` also succeeds for a loaded job that is not running: check the process.
ws_up() { pgrep -x WindowServer >/dev/null 2>&1; }
session_up() { pgrep -x -u "$USER_NAME" Finder >/dev/null 2>&1 && pgrep -x -u "$USER_NAME" Dock >/dev/null 2>&1; }
as_user() { launchctl asuser "$(id -u "$USER_NAME")" sudo -H -u "$USER_NAME" "$@"; }

wallpaper() {
    as_user osascript -l JavaScript -e 'ObjC.import("AppKit");
        ObjC.unwrap($.NSWorkspace.sharedWorkspace.desktopImageURLForScreen($.NSScreen.mainScreen).path)' 2>/dev/null
}

# reims' manual: reduce transparency/motion and a static wallpaper cut compositing work. Applied as
# the GUI user inside its settled session. Best effort: the desktop works without them, so failures
# only warn (on stderr, which `wsm gui` shows).
apply_user_preferences() {
    for pref in reduceTransparency reduceMotion; do
        as_user defaults write com.apple.universalaccess "$pref" -bool true 2>/dev/null \
            || echo "warning: could not set $pref for $USER_NAME" >&2
    done
    pic=""
    for p in "/System/Library/Desktop Pictures/Solid Colors/"*; do [ -f "$p" ] && { pic=$p; break; }; done
    [ -n "$pic" ] || { echo "warning: no solid-colour desktop picture found" >&2; return 0; }
    # Public API (NSWorkspace setDesktopImageURL) through JavaScript for Automation: no compiler
    # needed. Set right after Finder appears it can be dropped, so read it back and retry.
    i=0
    while [ $i -lt 10 ]; do
        as_user env WSM_PIC="$pic" osascript -l JavaScript -e 'ObjC.import("AppKit");
            var url = $.NSURL.fileURLWithPath($.NSProcessInfo.processInfo.environment.objectForKey("WSM_PIC"));
            var screens = $.NSScreen.screens;
            for (var i = 0; i < screens.count; i++)
                $.NSWorkspace.sharedWorkspace.setDesktopImageURLForScreenOptionsError(url, screens.objectAtIndex(i), $({}), null);
            "ok"' >/dev/null 2>&1
        sleep 2
        [ "$(wallpaper)" = "$pic" ] && return 0
        i=$((i + 1))
    done
    echo "warning: static wallpaper did not stick" >&2
}

bring_up_windowserver() {
    ws_up && return 0
    for l in com.apple.WindowServer com.apple.loginwindow com.apple.logind; do
        launchctl enable "system/$l" 2>/dev/null
        p=/System/Library/LaunchDaemons/$l.plist
        [ -f "$p" ] && launchctl bootstrap system "$p" 2>/dev/null
    done
    launchctl kickstart "$WS" 2>/dev/null
}

start_session() {
    session_up && return 0
    bring_up_windowserver
    defaults write /Library/Preferences/com.apple.loginwindow autoLoginUser "$USER_NAME"
    # loginwindow skips autologin when its lastUserName differs from the autologin user
    defaults write /Library/Preferences/com.apple.loginwindow lastUserName "$USER_NAME"
    # a session needs the loginwindow to (re)evaluate autologin
    pkill -x loginwindow 2>/dev/null
    i=0
    while [ $i -lt 120 ]; do
        if session_up; then
            sleep 3   # let Dock/WallpaperAgent settle before changing preferences
            apply_user_preferences
            return 0
        fi
        sleep 0.5; i=$((i + 1))
    done
    echo "desktop session for $USER_NAME not up after 60 s" >&2
    return 1
}

case "${1:-status}" in
    prewarm)
        have_gpu || { echo "no paravirt GPU"; exit 0; }
        start_session && echo "session prewarmed"
        ;;
    start)
        have_gpu || { echo "no paravirt GPU" >&2; exit 1; }
        start_session || exit 1
        echo "session up"
        ;;
    stop)
        echo "window hidden; session kept warm (wsm gui logout ends it)"
        ;;
    logout)
        defaults delete /Library/Preferences/com.apple.loginwindow autoLoginUser 2>/dev/null
        launchctl bootout "gui/$(id -u "$USER_NAME")" 2>/dev/null
        echo "logged out"
        ;;
    status)
        ws_up && echo "windowserver: up" || echo "windowserver: down"
        session_up && echo "session: up" || echo "session: down"
        echo "wallpaper: $(wallpaper)"
        ;;
    *)
        echo "usage: wsm-gui {start|stop|logout|status|prewarm}" >&2; exit 2
        ;;
esac
