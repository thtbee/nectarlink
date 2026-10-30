#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Installs NectarLink on Linux, with everything it needs, and shows each step
# as it goes:
#
#   curl -fsSL https://github.com/alejandrokundrah/nectarlink/releases/latest/download/install.sh | sh
#
# It picks the package that fits this system (.deb with apt, .rpm with dnf
# or zypper, the tarball elsewhere), checks it against the release's
# SHA256SUMS.txt, installs it with its dependencies, and then sets up the
# parts that need administrator rights once: the phone as a webcam
# (v4l2loopback), the phone as a keyboard and mouse (/dev/uinput), and the
# firewall, for your local network only.
#
# Options, after `sh -s --` when piped:
#   --uninstall        Remove NectarLink and the system settings this added
#   --no-extras        Install the app only
#   --version X.Y.Z    Install that release instead of the latest
#   --package KIND     deb, rpm or tarball, instead of the one that fits
#   --help             Show this
#
# NECTARLINK_ASSETS_DIR=/path takes the release files from a folder instead
# of GitHub (for testing).

# Everything is in functions, called on the last line: piped into sh, the
# whole script has arrived before anything runs.

REPO="alejandrokundrah/nectarlink"
DATA_PORT=7718
MDNS_PORT=5353
LOCALSEND_PORT=53317
MIN_GLIBC="2.39"
WEBCAM_NAME="NectarLink Webcam"
VCAM_MODULES_CONF="/etc/modules-load.d/nectarlink-vcam.conf"
VCAM_MODPROBE_CONF="/etc/modprobe.d/nectarlink-vcam.conf"
UINPUT_RULE="/etc/udev/rules.d/70-nectarlink-uinput.rules"
UINPUT_MODULES_CONF="/etc/modules-load.d/nectarlink-uinput.conf"
PRIVATE_NETS="192.168.0.0/16 10.0.0.0/8 172.16.0.0/12"

STEP=0
STEPS=7

setup_output() {
    if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
        BOLD="$(printf '\033[1m')"
        DIM="$(printf '\033[2m')"
        GREEN="$(printf '\033[32m')"
        YELLOW="$(printf '\033[33m')"
        RED="$(printf '\033[31m')"
        RESET="$(printf '\033[0m')"
    else
        BOLD="" DIM="" GREEN="" YELLOW="" RED="" RESET=""
    fi
}

step() {
    STEP=$((STEP + 1))
    printf '\n%s[%s/%s] %s%s\n' "$BOLD" "$STEP" "$STEPS" "$1" "$RESET"
}

say() { printf '      %s\n' "$1"; }
good() { printf '      %s%s%s\n' "$GREEN" "$1" "$RESET"; }
note() { printf '      %s%s%s\n' "$YELLOW" "$1" "$RESET"; }

fail() {
    printf '\n%sCould not install NectarLink:%s %s\n' "$RED" "$RESET" "$1" >&2
    [ -n "${2:-}" ] && printf '%s\n' "$2" >&2
    exit 1
}

# Runs a command, showing it first, with its output indented beneath.
run() {
    printf '      %s$ %s%s\n' "$DIM" "$*" "$RESET"
    run_status=0
    # The status goes down the pipe on a line of its own, after the output.
    { "$@" </dev/null 2>&1 || run_status=$?; printf '\n__exit__ %s\n' "$run_status"; } | {
        while IFS= read -r line; do
            case "$line" in
                "__exit__ "*) exit "${line#__exit__ }" ;;
                "") ;;
                *) printf '        %s%s%s\n' "$DIM" "$line" "$RESET" ;;
            esac
        done
    }
}

# Writes the line $2 to the system file $1, showing both.
put_file() {
    printf '      %s$ write %s%s\n' "$DIM" "$1" "$RESET"
    printf '        %s%s%s\n' "$DIM" "$2" "$RESET"
    if [ -n "$SUDO" ]; then
        "$SUDO" mkdir -p "${1%/*}" && printf '%s\n' "$2" | "$SUDO" tee "$1" >/dev/null
    else
        mkdir -p "${1%/*}" && printf '%s\n' "$2" > "$1"
    fi
}

# `run` with administrator rights.
admin() {
    if [ -n "$SUDO" ]; then run "$SUDO" "$@"; else run "$@"; fi
}

usage() {
    cat <<EOF
Installs NectarLink on Linux, with everything it needs.

  curl -fsSL https://github.com/$REPO/releases/latest/download/install.sh | sh
  curl -fsSL https://github.com/$REPO/releases/latest/download/install.sh | sh -s -- [options]

Options:
  --uninstall        Remove NectarLink and the system settings this added
  --no-extras        Install the app only (no webcam, keyboard or firewall setup)
  --version X.Y.Z    Install that release instead of the latest
  --package KIND     deb, rpm or tarball, instead of the one that fits
  --help             Show this
EOF
}

parse_args() {
    UNINSTALL=0
    EXTRAS=1
    WANT_VERSION=""
    PACKAGE=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --uninstall) UNINSTALL=1 ;;
            --no-extras) EXTRAS=0 ;;
            --version) shift; WANT_VERSION="${1:-}"; [ -n "$WANT_VERSION" ] || fail "--version needs a version, such as 0.1.5." ;;
            --version=*) WANT_VERSION="${1#--version=}" ;;
            --package) shift; PACKAGE="${1:-}" ;;
            --package=*) PACKAGE="${1#--package=}" ;;
            -h|--help) usage; exit 0 ;;
            *) fail "unknown option $1" "Run with --help to see the options." ;;
        esac
        shift
    done
    WANT_VERSION="${WANT_VERSION#v}"
    case "$PACKAGE" in
        ""|deb|rpm|tarball) ;;
        *) fail "--package takes deb, rpm or tarball." ;;
    esac
}

# Whether version $1 is at least $2 (dotted numbers).
version_at_least() {
    [ "$(printf '%s\n%s\n' "$2" "$1" | sort -t. -k1,1n -k2,2n -k3,3n | head -n 1)" = "$2" ]
}

has() { command -v "$1" >/dev/null 2>&1; }

find_admin() {
    if [ "$(id -u)" -eq 0 ]; then
        SUDO=""
        HAVE_ADMIN=1
    elif has sudo; then
        SUDO="sudo"
        HAVE_ADMIN=1
    elif has doas; then
        SUDO="doas"
        HAVE_ADMIN=1
    else
        SUDO=""
        HAVE_ADMIN=0
    fi
}

# The user NectarLink runs as: whoever ran this, also through sudo.
find_user() {
    TARGET_USER="$(id -un)"
    TARGET_HOME="$HOME"
    if [ "$(id -u)" -eq 0 ] && [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ]; then
        TARGET_USER="$SUDO_USER"
        TARGET_HOME="$(getent passwd "$SUDO_USER" | cut -d: -f6)"
    fi
}

check_system() {
    step "Checking this computer"
    [ "$(uname -s)" = "Linux" ] || fail "this installer is for Linux." "On Windows, download the installer from https://github.com/$REPO/releases/latest"

    ARCH="$(uname -m)"
    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64"; DEB_ARCH="amd64" ;;
        aarch64|arm64) fail "NectarLink for Linux is built for 64-bit Intel and AMD PCs so far, not ARM." ;;
        *) fail "NectarLink for Linux needs a 64-bit Intel or AMD PC (this is $ARCH)." ;;
    esac

    OS_NAME="Linux"
    if [ -r /etc/os-release ]; then
        # shellcheck disable=SC1091
        OS_NAME="$(. /etc/os-release && printf '%s' "${PRETTY_NAME:-${NAME:-Linux}}")"
    fi

    GLIBC="$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')"
    [ -n "$GLIBC" ] || fail "NectarLink needs glibc, and this system uses another C library (such as musl on Alpine)."
    version_at_least "$GLIBC" "$MIN_GLIBC" || fail "this system is too old for NectarLink (glibc $GLIBC; it needs $MIN_GLIBC or newer)." \
        "It runs on Ubuntu 24.04 or newer, Debian 13, Fedora 40 or newer, and other distributions from 2024 on."

    good "$OS_NAME, $ARCH, glibc $GLIBC"

    if [ -z "$PACKAGE" ]; then
        if has apt-get && has dpkg; then
            PACKAGE="deb"
        elif has dnf || has zypper; then
            PACKAGE="rpm"
        else
            PACKAGE="tarball"
        fi
    fi
    find_admin
    find_user
    if [ "$HAVE_ADMIN" -eq 0 ]; then
        if [ "$PACKAGE" != "tarball" ]; then
            note "Neither sudo nor doas is here, so NectarLink goes into your home folder instead."
            PACKAGE="tarball"
        fi
        if [ "$EXTRAS" -eq 1 ]; then
            note "Without administrator rights, the webcam, keyboard and firewall setup is skipped."
            EXTRAS=0
        fi
    fi
    case "$PACKAGE" in
        deb) say "Package: .deb, installed with apt" ;;
        rpm) if has dnf; then say "Package: .rpm, installed with dnf"; else say "Package: .rpm, installed with zypper"; fi ;;
        tarball) say "Package: the portable tarball" ;;
    esac
    if [ "$HAVE_ADMIN" -eq 1 ] && [ -n "$SUDO" ]; then
        say "Some steps need administrator rights; $SUDO may ask for your password."
        # Asked once, up front, rather than in the middle of a step. sudo
        # reads the password from the terminal, not from the piped script.
        "$SUDO" true || fail "administrator rights are needed to install NectarLink."
    fi
}

# Fetches $1 to file $2.
fetch() {
    if [ -n "${NECTARLINK_ASSETS_DIR:-}" ]; then
        cp "$NECTARLINK_ASSETS_DIR/${1##*/}" "$2"
    elif has curl; then
        curl -fsSL --retry 3 -o "$2" "$1"
    elif has wget; then
        wget -q -O "$2" "$1"
    else
        fail "this needs curl or wget to download NectarLink." "Install one of them and run this again."
    fi
}

find_release() {
    step "Finding the release"
    if [ -n "$WANT_VERSION" ]; then
        VERSION="$WANT_VERSION"
    elif [ -n "${NECTARLINK_ASSETS_DIR:-}" ]; then
        for f in "$NECTARLINK_ASSETS_DIR"/Nectarlink-*-linux-x86_64.tar.gz; do
            [ -f "$f" ] || continue
            VERSION="${f##*/Nectarlink-}"
            VERSION="${VERSION%-linux-x86_64.tar.gz}"
        done
    else
        fetch "https://api.github.com/repos/$REPO/releases/latest" "$WORK/release.json" \
            || fail "couldn't reach GitHub to find the latest release." "Check the internet connection and try again."
        VERSION="$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' "$WORK/release.json" | head -n 1)"
    fi
    [ -n "$VERSION" ] || fail "couldn't tell which version is the latest."
    BASE_URL="https://github.com/$REPO/releases/download/v$VERSION"
    case "$PACKAGE" in
        deb) ASSET="nectarlink_${VERSION}_${DEB_ARCH}.deb" ;;
        rpm) ASSET="nectarlink-${VERSION}-1.${ARCH}.rpm" ;;
        tarball) ASSET="Nectarlink-${VERSION}-linux-${ARCH}.tar.gz" ;;
    esac
    good "NectarLink $VERSION"
}

download() {
    step "Downloading $ASSET"
    say "From $BASE_URL"
    fetch "$BASE_URL/$ASSET" "$WORK/$ASSET" || fail "couldn't download $ASSET." \
        "If this release has no such file yet, try --package tarball."
    fetch "$BASE_URL/SHA256SUMS.txt" "$WORK/SHA256SUMS.txt" || fail "couldn't download SHA256SUMS.txt to check the download."
    size="$(du -h "$WORK/$ASSET" | cut -f1)"
    say "Downloaded $size"
    expected="$(awk -v f="$ASSET" '$2 == f || $2 == "*"f {print $1}' "$WORK/SHA256SUMS.txt" | head -n 1)"
    [ -n "$expected" ] || fail "$ASSET isn't listed in SHA256SUMS.txt, so it can't be checked."
    actual="$(sha256sum "$WORK/$ASSET" | cut -d' ' -f1)"
    [ "$actual" = "$expected" ] || fail "the download doesn't match SHA256SUMS.txt; it may be damaged or tampered with." \
        "Nothing was installed. Try again in a moment."
    good "Checksum matches SHA256SUMS.txt"
    # Readable by the package manager's own user (apt's _apt).
    chmod 755 "$WORK"
    chmod 644 "$WORK/$ASSET"
}

install_tarball_dependencies() {
    if has pacman; then
        admin pacman -S --needed --noconfirm libglvnd libx11 libxcb libxkbcommon wayland fontconfig freetype2 dbus \
            || note "Some libraries didn't install; NectarLink may not start until they do."
    else
        say "Make sure OpenGL, X11 or Wayland, fontconfig, freetype and D-Bus libraries are installed."
    fi
}

install_app() {
    step "Installing NectarLink"
    case "$PACKAGE" in
        deb)
            admin env DEBIAN_FRONTEND=noninteractive apt-get update || note "Couldn't refresh the package lists; trying with what's there."
            admin env DEBIAN_FRONTEND=noninteractive apt-get install -y "$WORK/$ASSET" \
                || fail "apt couldn't install the package." "The messages above say why."
            ;;
        rpm)
            if has dnf; then
                admin dnf install -y --nogpgcheck "$WORK/$ASSET" || fail "dnf couldn't install the package." "The messages above say why."
            else
                admin zypper --non-interactive --no-gpg-checks install "$WORK/$ASSET" \
                    || fail "zypper couldn't install the package." "The messages above say why."
            fi
            ;;
        tarball)
            run tar -xzf "$WORK/$ASSET" -C "$WORK" || fail "couldn't unpack $ASSET."
            if [ "$HAVE_ADMIN" -eq 1 ]; then
                install_tarball_dependencies
                admin "$WORK/Nectarlink-$VERSION-linux-$ARCH/install.sh" || fail "the tarball's installer failed." "The messages above say why."
            else
                run "$WORK/Nectarlink-$VERSION-linux-$ARCH/install.sh" || fail "the tarball's installer failed." "The messages above say why."
                case ":$PATH:" in
                    *":$HOME/.local/bin:"*) ;;
                    *) note "Add ~/.local/bin to your PATH to start it with nectarlink-desktop from a terminal." ;;
                esac
            fi
            ;;
    esac
    good "NectarLink $VERSION is installed, with its entry in your app menu"
}

# Whether the kernel can load modules here (not in most containers).
can_load_modules() {
    [ -d "/lib/modules/$(uname -r)" ] && has modprobe
}

setup_webcam() {
    step "Phone as webcam"
    if [ "$EXTRAS" -eq 0 ]; then say "Skipped."; return 0; fi
    if ! can_load_modules; then
        note "Skipped: this system can't load kernel modules (a container?)."
        return 0
    fi
    if ! modinfo v4l2loopback >/dev/null 2>&1; then
        say "Installing the v4l2loopback driver, which gives apps the NectarLink Webcam"
        if has apt-get; then
            admin env DEBIAN_FRONTEND=noninteractive apt-get install -y "linux-headers-$(uname -r)" \
                || note "No kernel headers for $(uname -r); the driver may not build."
            admin env DEBIAN_FRONTEND=noninteractive apt-get install -y v4l2loopback-dkms || true
        elif has dnf; then
            admin dnf install -y v4l2loopback || true
        elif has zypper; then
            admin zypper --non-interactive install v4l2loopback-kmp-default || true
        elif has pacman; then
            admin pacman -S --needed --noconfirm v4l2loopback-dkms || true
        fi
    fi
    if ! modinfo v4l2loopback >/dev/null 2>&1; then
        if has dnf; then
            note "Fedora gets v4l2loopback from RPM Fusion. To use the phone as a webcam, enable it"
            note "(https://rpmfusion.org/Configuration), then run: sudo dnf install v4l2loopback"
        else
            note "v4l2loopback isn't available for this kernel; the phone as webcam won't work until it is."
        fi
        return 0
    fi
    put_file "$VCAM_MODULES_CONF" "v4l2loopback"
    put_file "$VCAM_MODPROBE_CONF" "options v4l2loopback exclusive_caps=1 card_label=\"$WEBCAM_NAME\""
    if [ -d /sys/module/v4l2loopback ]; then
        good "Ready: the driver is loaded and starts with the computer"
        return 0
    fi
    if admin modprobe v4l2loopback exclusive_caps=1 "card_label=$WEBCAM_NAME"; then
        good "Ready: apps list the NectarLink Webcam"
    elif has mokutil && mokutil --sb-state 2>/dev/null | grep -q enabled; then
        note "Secure Boot blocks the new driver until its key is enrolled. Run:"
        note "  sudo mokutil --import /var/lib/dkms/mok.pub"
        note "then restart and choose Enroll MOK. After that, the webcam works."
    else
        note "The driver didn't load now; it will after a restart."
    fi
}

setup_input() {
    step "Phone as keyboard and mouse"
    if [ "$EXTRAS" -eq 0 ]; then say "Skipped."; return 0; fi
    if [ ! -d /etc/udev ]; then
        note "Skipped: this system has no udev."
        return 0
    fi
    # The signed-in user (not every account) may create input devices, as
    # with game controller drivers.
    put_file "$UINPUT_RULE" 'KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"'
    put_file "$UINPUT_MODULES_CONF" "uinput"
    if can_load_modules; then
        admin modprobe uinput || true
    fi
    if has udevadm && [ -d /run/udev ]; then
        admin udevadm control --reload-rules || true
        admin udevadm trigger --sysname-match=uinput || true
    fi
    good "Ready: your phone can type and move the pointer here"
}

# Prints the commands that open (or with $1 = delete, close) the ports.
ufw_rules() {
    for net in $PRIVATE_NETS fe80::/10; do
        for port in $DATA_PORT $LOCALSEND_PORT; do
            echo "ufw $1 allow from $net to any port $port proto tcp"
            echo "ufw $1 allow from $net to any port $port proto udp"
        done
        echo "ufw $1 allow from $net to any port $MDNS_PORT proto udp"
    done
}

firewalld_rules() {
    for net in $PRIVATE_NETS; do
        for port in $DATA_PORT $LOCALSEND_PORT; do
            for proto in tcp udp; do
                echo "firewall-cmd --permanent --$1-rich-rule='rule family=\"ipv4\" source address=\"$net\" port port=\"$port\" protocol=\"$proto\" accept'"
            done
        done
        echo "firewall-cmd --permanent --$1-rich-rule='rule family=\"ipv4\" source address=\"$net\" port port=\"$MDNS_PORT\" protocol=\"udp\" accept'"
    done
    for port in $DATA_PORT $LOCALSEND_PORT; do
        for proto in tcp udp; do
            echo "firewall-cmd --permanent --$1-rich-rule='rule family=\"ipv6\" source address=\"fe80::/10\" port port=\"$port\" protocol=\"$proto\" accept'"
        done
    done
    echo "firewall-cmd --permanent --$1-rich-rule='rule family=\"ipv6\" source address=\"fe80::/10\" port port=\"$MDNS_PORT\" protocol=\"udp\" accept'"
}

firewall_kind() {
    if has ufw && $SUDO ufw status 2>/dev/null </dev/null | grep -q '^Status: active'; then
        echo ufw
    elif has firewall-cmd && firewall-cmd --state >/dev/null 2>&1 </dev/null; then
        echo firewalld
    fi
}

# Tells the app the firewall was opened (it can't read the rules itself).
remember_firewall() {
    [ -n "$TARGET_HOME" ] || return 0
    dir="${XDG_DATA_HOME:-$TARGET_HOME/.local/share}/Nectarlink"
    if [ "$(id -u)" -eq 0 ] && [ "$TARGET_USER" != "root" ]; then
        su -s /bin/sh "$TARGET_USER" -c "mkdir -p '$dir' && printf '$DATA_PORT\n' > '$dir/firewall_verified'" 2>/dev/null || true
    elif [ "$(id -u)" -ne 0 ]; then
        if mkdir -p "$dir"; then printf '%s\n' "$DATA_PORT" > "$dir/firewall_verified" || true; fi
    fi
}

setup_firewall() {
    step "Firewall"
    if [ "$EXTRAS" -eq 0 ]; then say "Skipped."; return 0; fi
    case "$(firewall_kind)" in
        ufw)
            say "Opening NectarLink's ports to your local network only"
            ufw_rules "" | while IFS= read -r rule; do
                # IPv6 rules fail where ufw has IPv6 turned off; that's fine.
                # shellcheck disable=SC2086
                admin $rule || case "$rule" in *fe80::*) ;; *) exit 1 ;; esac
            done || { note "Some rules didn't apply; NectarLink's Settings can fix this later."; return 0; }
            remember_firewall
            good "Ready: phones on your network can reach this PC"
            ;;
        firewalld)
            say "Opening NectarLink's ports to your local network only"
            firewalld_rules add > "$WORK/firewalld.sh"
            echo "firewall-cmd --reload" >> "$WORK/firewalld.sh"
            admin sh -e "$WORK/firewalld.sh" || { note "Some rules didn't apply; NectarLink's Settings can fix this later."; return 0; }
            remember_firewall
            good "Ready: phones on your network can reach this PC"
            ;;
        *)
            good "No firewall is on, so there's nothing to open"
            ;;
    esac
}

finish() {
    printf '\n%sNectarLink %s is ready.%s\n' "$BOLD" "$VERSION" "$RESET"
    echo "Open it from your app menu, or run nectarlink-desktop. Then install the"
    echo "Android app and pair: https://github.com/$REPO#get-started"
    echo "It updates itself from now on. If it was already open, quit and open it again."
}

uninstall() {
    STEPS=3
    find_admin
    find_user
    step "Removing NectarLink"
    removed=0
    if has dpkg && dpkg -s nectarlink >/dev/null 2>&1; then
        admin env DEBIAN_FRONTEND=noninteractive apt-get remove -y nectarlink && removed=1
    fi
    if has rpm && rpm -q --quiet nectarlink 2>/dev/null; then
        if has dnf; then admin dnf remove -y nectarlink && removed=1
        elif has zypper; then admin zypper --non-interactive remove nectarlink && removed=1
        else admin rpm -e nectarlink && removed=1
        fi
    fi
    if [ -d /opt/nectarlink ]; then
        admin rm -rf /opt/nectarlink /usr/local/bin/nectarlink-desktop /usr/share/applications/nectarlink.desktop \
            /usr/share/metainfo/org.nectarlink.desktop.metainfo.xml /usr/share/icons/hicolor/scalable/apps/nectarlink.svg \
            /usr/share/icons/hicolor/512x512/apps/nectarlink.png && removed=1
    fi
    user_share="${XDG_DATA_HOME:-$TARGET_HOME/.local/share}"
    if [ -d "$user_share/nectarlink-app" ]; then
        run rm -rf "$user_share/nectarlink-app" "$TARGET_HOME/.local/bin/nectarlink-desktop" \
            "$user_share/metainfo/org.nectarlink.desktop.metainfo.xml" && removed=1
    fi
    # The menu entry and icon the app keeps for itself, and starting with the computer.
    run rm -f "$user_share/applications/nectarlink.desktop" "$user_share/icons/hicolor/256x256/apps/nectarlink.png" \
        "$user_share/icons/hicolor/512x512/apps/nectarlink.png" "$user_share/icons/hicolor/scalable/apps/nectarlink.svg" \
        "${XDG_CONFIG_HOME:-$TARGET_HOME/.config}/autostart/nectarlink.desktop"
    if [ "$removed" -eq 1 ]; then good "The app is removed"; else say "The app wasn't installed by a package or the tarball."; fi

    step "Removing the system settings"
    if [ "$HAVE_ADMIN" -eq 1 ]; then
        admin rm -f "$VCAM_MODULES_CONF" "$VCAM_MODPROBE_CONF" "$UINPUT_RULE" "$UINPUT_MODULES_CONF"
        if has udevadm && [ -d /run/udev ]; then admin udevadm control --reload-rules || true; fi
        good "Webcam and keyboard settings are removed"
    else
        note "Without administrator rights, settings in /etc stay."
    fi

    step "Closing the firewall"
    case "$(firewall_kind)" in
        ufw)
            ufw_rules delete | while IFS= read -r rule; do
                # shellcheck disable=SC2086
                admin $rule || true
            done
            good "NectarLink's ports are closed again"
            ;;
        firewalld)
            firewalld_rules remove | sed 's/$/ || true/' > "$WORK/firewalld.sh"
            echo "firewall-cmd --reload" >> "$WORK/firewalld.sh"
            admin sh "$WORK/firewalld.sh" || true
            good "NectarLink's ports are closed again"
            ;;
        *) say "No firewall is on." ;;
    esac

    printf '\n%sNectarLink is removed.%s\n' "$BOLD" "$RESET"
    echo "Your settings and paired devices are still in ${user_share}/Nectarlink;"
    echo "delete that folder to remove them too."
}

main() {
    # modprobe, modinfo and udevadm live in sbin, which isn't always on a user's PATH.
    PATH="$PATH:/usr/sbin:/sbin"
    setup_output
    parse_args "$@"
    WORK="$(mktemp -d "${TMPDIR:-/tmp}/nectarlink-install.XXXXXX")" || fail "couldn't create a temporary folder."
    trap 'rm -rf "$WORK"' EXIT
    trap 'exit 130' INT TERM

    if [ "$UNINSTALL" -eq 1 ]; then
        printf '%sNectarLink uninstaller%s\n' "$BOLD" "$RESET"
        uninstall
        return 0
    fi

    printf '%sNectarLink installer%s\n' "$BOLD" "$RESET"
    check_system
    find_release
    download
    install_app
    setup_webcam
    setup_input
    setup_firewall
    finish
}

main "$@"
