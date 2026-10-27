#!/usr/bin/env bash
# =============================================================================
#  LisekMovie installer for Linux
# =============================================================================
#
#  Supported: Debian, Ubuntu, Arch Linux (other distributions: at your own
#  risk, with apt or pacman). Three ways to install:
#
#    1) build from source      Rust + Node.js toolchains, release build,
#                              systemd services, local PostgreSQL
#    2) Docker Compose         the stack from docker-compose.yml (recommended)
#    3) release binaries       the latest package from GitHub Releases,
#                              systemd services, local PostgreSQL
#
#  Usage:
#    curl -fsSLO https://raw.githubusercontent.com/LisekMovieApp/LisekMovie-app/main/install-linux.sh
#    sudo bash install-linux.sh
#
#  Non-interactive: LISEK_LANG=en|pl LISEK_METHOD=source|docker|release
#  LISEK_ASSUME_YES=1 sudo -E bash install-linux.sh
#
#  Paths (override with environment variables):
#    LISEK_PREFIX    /opt/lisekmovie       program, sources, toolchains
#    LISEK_DATA_DIR  /var/lib/lisekmovie   .env, posters, thumbnails, cache
#                                          (native installs; Docker keeps
#                                          its data in named volumes)
#  Updates: sudo bash /opt/lisekmovie/update-linux.sh
# =============================================================================

set -Eeuo pipefail
umask 022

# --- settings -----------------------------------------------------------------
GITHUB_OWNER="${LISEK_GITHUB_OWNER:-LisekMovieApp}"
GIT_BASE="${LISEK_GIT_BASE:-https://github.com/${GITHUB_OWNER}}"
GIT_REF="${LISEK_REF:-main}"
# The newest release, pre-releases included (/releases/latest skips them).
# Stable releases only: LISEK_RELEASE_API=https://api.github.com/repos/LisekMovieApp/LisekMovie-app/releases/latest
RELEASE_API="${LISEK_RELEASE_API:-https://api.github.com/repos/${GITHUB_OWNER}/LisekMovie-app/releases?per_page=1}"
PREFIX="${LISEK_PREFIX:-/opt/lisekmovie}"
DATA_DIR="${LISEK_DATA_DIR:-/var/lib/lisekmovie}"
SERVICE_USER="${LISEK_USER:-lisek}"
HTTP_PORT="${LISEK_HTTP_PORT:-8096}"
NODE_MAJOR=24
MIN_NODE_MAJOR=20
# The app and every metadata provider extension, cloned side by side (the
# backend builds them as path dependencies).
REPOS=(
    LisekMovie-app
    LisekMovie-plugin-tmdb LisekMovie-plugin-thetvdb
    LisekMovie-plugin-imdb LisekMovie-plugin-omdb
    LisekMovie-plugin-anidb LisekMovie-plugin-anilist LisekMovie-plugin-anisearch
    LisekMovie-plugin-imvdb
    LisekMovie-plugin-vgmdb LisekMovie-plugin-audiodb
)
ASSUME_YES="${LISEK_ASSUME_YES:-0}"
LANG_CODE="${LISEK_LANG:-}"
METHOD="${LISEK_METHOD:-}"
SRC_DIR="${PREFIX}/src"
APP_DIR="${PREFIX}/app"
CONF_FILE="${PREFIX}/install.conf"
LOG_FILE="${LISEK_LOG:-/tmp/lisekmovie-install.log}"

# --- output ---------------------------------------------------------------------
if [[ -t 1 ]] && [[ "${NO_COLOR:-}" == "" ]]; then
    C_RESET=$'\e[0m' C_BOLD=$'\e[1m' C_DIM=$'\e[2m' C_RED=$'\e[31m' C_GREEN=$'\e[32m'
    C_YELLOW=$'\e[33m' C_BLUE=$'\e[34m' C_GOLD=$'\e[38;5;179m'
else
    C_RESET='' C_BOLD='' C_DIM='' C_RED='' C_GREEN='' C_YELLOW='' C_BLUE='' C_GOLD=''
fi

info() { printf '%s➜%s %s\n' "$C_BLUE" "$C_RESET" "$*"; }
ok() { printf '%s✔%s %s\n' "$C_GREEN" "$C_RESET" "$*"; }
warn() { printf '%s!%s %s\n' "$C_YELLOW" "$C_RESET" "$*" >&2; }
fail() { printf '%s✖ %s%s\n' "$C_RED" "$*" "$C_RESET" >&2; exit 1; }
step() { printf '\n%s%s== %s ==%s\n' "$C_BOLD" "$C_GOLD" "$*" "$C_RESET"; }

banner() {
    printf '%s' "$C_GOLD"
    cat <<'EOF'
   _      _          _    __  __            _
  | |    (_)___  ___| | _|  \/  | _____   _(_) ___
  | |    | / __|/ _ \ |/ / |\/| |/ _ \ \ / / |/ _ \
  | |___ | \__ \  __/   <| |  | | (_) \ V /| |  __/
  |_____||_|___/\___|_|\_\_|  |_|\___/ \_/ |_|\___|
EOF
    printf '%s%s  Self-hosted video on demand · Rust + SvelteKit%s\n\n' "$C_RESET" "$C_DIM" "$C_RESET"
}

# --- translations ------------------------------------------------------------------
declare -A EN=(
    [need_root]="Run the installer as root: sudo bash install-linux.sh"
    [os_ok]="Detected %s: supported."
    [os_other]="%s is not officially supported yet. Debian, Ubuntu and Arch Linux are; support for more distributions will come in a future release."
    [os_continue]="Continue at your own risk?"
    [os_no_pm]="Neither apt-get nor pacman was found: this distribution cannot be installed automatically."
    [os_unknown]="Cannot read /etc/os-release: the distribution is unknown."
    [aborted]="Installation cancelled. Nothing was changed."
    [existing]="LisekMovie is already installed here (%s, %s). To update it, run: sudo bash %s/update-linux.sh"
    [existing_continue]="Install again? Your .env, database and media data are kept."
    [method_title]="Choose how to install LisekMovie:"
    [method_1]="Compile from scratch (Build from source)"
    [method_2]="Install via Docker Compose (Recommended)"
    [method_3]="Download latest release (Binaries)"
    [choice]="Your choice"
    [invalid]="Invalid choice, try again."
    [pacman_hint]="pacman could not install the packages. Update the system first (pacman -Syu), then run the installer again."
    [db_failed]="Creating the database failed. Details: %s"
    [no_input]="No answer (input closed). Run the script in a terminal, or set LISEK_LANG, LISEK_METHOD and LISEK_ASSUME_YES=1."
    [yes_no]="[Y/n]"
    [no_yes]="[y/N]"
    [step_packages]="System packages"
    [step_sources]="Source code"
    [step_rust]="Rust toolchain"
    [step_node]="Node.js"
    [step_build]="Release build"
    [step_db]="PostgreSQL"
    [step_service]="Services"
    [step_docker]="Docker"
    [step_release]="Latest release"
    [installing]="Installing: %s"
    [cloning]="Cloning %s"
    [fetching]="Updating %s"
    [rust_install]="Installing Rust (rustup, stable) into %s"
    [rust_found]="Rust: %s"
    [node_found]="Node.js %s"
    [node_old]="Node.js %s is too old (needs %s+); installing Node.js %s."
    [build_backend]="Building lisek-server (cargo build --release); this takes several minutes the first time."
    [build_frontend]="Building the web app (npm ci, vite build)."
    [build_done]="Build ready: %s"
    [db_setup]="Set up a local PostgreSQL database for LisekMovie?"
    [db_created]="Database 'lisekmovie' and role 'lisekmovie' ready."
    [db_exists]="Database role 'lisekmovie' already exists: keeping it. Its password is in the DATABASE_URL of your previous .env; otherwise enter the database in the first-run wizard."
    [db_skip]="Skipped. Enter the database address in the first-run wizard."
    [env_created]="Created %s from .env.example."
    [env_kept]="Kept the existing %s."
    [media_dir]="Folder with your films and series"
    [http_port]="Port of the web app"
    [user_created]="Created system user '%s'."
    [systemd_missing]="systemd is not running here: start LisekMovie by hand:"
    [service_started]="Services lisekmovie-server and lisekmovie-web started."
    [health_wait]="Waiting for the server to answer..."
    [health_ok]="The server is up."
    [health_fail]="The server did not answer in time. Check: %s"
    [docker_install]="Docker or the Compose plugin is missing: installing it."
    [docker_found]="Docker %s with Compose %s"
    [docker_gpu_nvidia]="An NVIDIA GPU was found. Use it for transcoding (needs the NVIDIA Container Toolkit)?"
    [docker_gpu_intel]="An Intel/AMD GPU (/dev/dri) was found. Pass it to the server for Quick Sync / VA-API?"
    [docker_up]="Starting the stack: docker compose up -d --build (the first build takes a while)."
    [docker_running]="Containers are running."
    [release_query]="Asking GitHub for the latest release..."
    [release_none]="No release package for linux-%s was found. Choose 'Compile from scratch' or 'Docker Compose' instead."
    [release_found]="Latest release: %s"
    [release_download]="Downloading %s"
    [checksum_ok]="Checksum verified."
    [checksum_bad]="Checksum mismatch: the download is damaged or was tampered with. Nothing was installed."
    [checksum_missing]="The release has no .sha256 file: the download cannot be verified."
    [done_title]="LisekMovie is installed!"
    [done_open]="Open %s in your browser to create the administrator account."
    [done_data]="Your data: %s (never touched by updates)."
    [done_update]="Update later with: sudo bash %s/update-linux.sh"
    [done_log]="Full log: %s"
    [error_at]="Installation failed (line %s). See the log: %s"
)
declare -A PL=(
    [need_root]="Uruchom instalator jako root: sudo bash install-linux.sh"
    [os_ok]="Wykryto %s: wspierany."
    [os_other]="%s nie jest jeszcze oficjalnie wspierany. Wspierane są Debian, Ubuntu i Arch Linux; obsługa kolejnych dystrybucji pojawi się w przyszłych wersjach."
    [os_continue]="Kontynuować na własne ryzyko?"
    [os_no_pm]="Nie znaleziono ani apt-get, ani pacman: tej dystrybucji nie da się zainstalować automatycznie."
    [os_unknown]="Nie można odczytać /etc/os-release: nieznana dystrybucja."
    [aborted]="Instalacja anulowana. Nic nie zostało zmienione."
    [existing]="LisekMovie jest już tu zainstalowany (%s, %s). Aby go zaktualizować, uruchom: sudo bash %s/update-linux.sh"
    [existing_continue]="Zainstalować ponownie? Plik .env, baza danych i dane mediów zostaną zachowane."
    [method_title]="Wybierz sposób instalacji LisekMovie:"
    [method_1]="Wykompiluj od zera (ze źródeł)"
    [method_2]="Zainstaluj przez Docker Compose (Zalecane)"
    [method_3]="Pobierz najnowszą gotową paczkę (Release)"
    [choice]="Twój wybór"
    [invalid]="Nieprawidłowy wybór, spróbuj ponownie."
    [pacman_hint]="pacman nie zainstalował pakietów. Najpierw zaktualizuj system (pacman -Syu), potem uruchom instalator ponownie."
    [db_failed]="Nie udało się utworzyć bazy danych. Szczegóły: %s"
    [no_input]="Brak odpowiedzi (wejście zamknięte). Uruchom skrypt w terminalu albo ustaw LISEK_LANG, LISEK_METHOD i LISEK_ASSUME_YES=1."
    [yes_no]="[T/n]"
    [no_yes]="[t/N]"
    [step_packages]="Pakiety systemowe"
    [step_sources]="Kod źródłowy"
    [step_rust]="Rust"
    [step_node]="Node.js"
    [step_build]="Kompilacja (release)"
    [step_db]="PostgreSQL"
    [step_service]="Usługi"
    [step_docker]="Docker"
    [step_release]="Najnowsze wydanie"
    [installing]="Instaluję: %s"
    [cloning]="Klonuję %s"
    [fetching]="Aktualizuję %s"
    [rust_install]="Instaluję Rust (rustup, stable) w %s"
    [rust_found]="Rust: %s"
    [node_found]="Node.js %s"
    [node_old]="Node.js %s jest za stary (wymagany %s+); instaluję Node.js %s."
    [build_backend]="Kompiluję lisek-server (cargo build --release); za pierwszym razem trwa to kilka minut."
    [build_frontend]="Buduję aplikację webową (npm ci, vite build)."
    [build_done]="Kompilacja gotowa: %s"
    [db_setup]="Skonfigurować lokalną bazę PostgreSQL dla LisekMovie?"
    [db_created]="Baza 'lisekmovie' i rola 'lisekmovie' gotowe."
    [db_exists]="Rola 'lisekmovie' już istnieje: zostaje bez zmian. Jej hasło jest w DATABASE_URL poprzedniego .env; w przeciwnym razie podaj bazę w kreatorze pierwszego uruchomienia."
    [db_skip]="Pominięto. Adres bazy podasz w kreatorze pierwszego uruchomienia."
    [env_created]="Utworzono %s z .env.example."
    [env_kept]="Zachowano istniejący %s."
    [media_dir]="Folder z filmami i serialami"
    [http_port]="Port aplikacji webowej"
    [user_created]="Utworzono użytkownika systemowego '%s'."
    [systemd_missing]="systemd tu nie działa: uruchom LisekMovie ręcznie:"
    [service_started]="Usługi lisekmovie-server i lisekmovie-web uruchomione."
    [health_wait]="Czekam na odpowiedź serwera..."
    [health_ok]="Serwer działa."
    [health_fail]="Serwer nie odpowiedział na czas. Sprawdź: %s"
    [docker_install]="Brak Dockera lub wtyczki Compose: instaluję."
    [docker_found]="Docker %s z Compose %s"
    [docker_gpu_nvidia]="Wykryto kartę NVIDIA. Użyć jej do transkodowania (wymaga NVIDIA Container Toolkit)?"
    [docker_gpu_intel]="Wykryto GPU Intel/AMD (/dev/dri). Przekazać je serwerowi dla Quick Sync / VA-API?"
    [docker_up]="Uruchamiam: docker compose up -d --build (pierwsza kompilacja trochę trwa)."
    [docker_running]="Kontenery działają."
    [release_query]="Sprawdzam najnowsze wydanie na GitHubie..."
    [release_none]="Nie znaleziono paczki dla linux-%s. Wybierz 'Wykompiluj od zera' albo 'Docker Compose'."
    [release_found]="Najnowsze wydanie: %s"
    [release_download]="Pobieram %s"
    [checksum_ok]="Suma kontrolna poprawna."
    [checksum_bad]="Suma kontrolna się nie zgadza: plik jest uszkodzony lub podmieniony. Nic nie zostało zainstalowane."
    [checksum_missing]="Wydanie nie ma pliku .sha256: nie da się zweryfikować pobranego pliku."
    [done_title]="LisekMovie zainstalowany!"
    [done_open]="Otwórz %s w przeglądarce i utwórz konto administratora."
    [done_data]="Twoje dane: %s (aktualizacje ich nie ruszają)."
    [done_update]="Aktualizacja: sudo bash %s/update-linux.sh"
    [done_log]="Pełny log: %s"
    [error_at]="Instalacja nie powiodła się (linia %s). Zobacz log: %s"
)

# t KEY [printf args...]
t() {
    local key=$1
    shift
    local text
    if [[ "$LANG_CODE" == "pl" ]]; then text=${PL[$key]:-${EN[$key]:-$key}}; else text=${EN[$key]:-$key}; fi
    # shellcheck disable=SC2059
    printf -- "$text" "$@"
}

# --- input (works when the script itself comes from a pipe) ---------------------------
TTY=/dev/tty
# read_line -> REPLY_LINE. Reads the terminal even when the script itself comes from a pipe;
# end of input (no terminal, closed stdin) stops the script instead of looping on a prompt.
REPLY_LINE=""
read_line() {
    REPLY_LINE=""
    if [[ -r "$TTY" ]] && { : <"$TTY"; } 2>/dev/null; then
        IFS= read -r REPLY_LINE <"$TTY" && return 0
    else
        IFS= read -r REPLY_LINE && return 0
    fi
    [[ -n "$REPLY_LINE" ]] && return 0
    printf '\n' >&2
    fail "$(t no_input)"
}

# ask_yes_no KEY default(y|n)
ask_yes_no() {
    local question default=$2 hint reply
    question=$(t "$1")
    if [[ "$ASSUME_YES" == "1" ]]; then [[ "$default" == "y" ]]; return; fi
    if [[ "$default" == "y" ]]; then hint=$(t yes_no); else hint=$(t no_yes); fi
    while true; do
        printf '%s?%s %s %s ' "$C_GOLD" "$C_RESET" "$question" "$hint"
        read_line
        reply=${REPLY_LINE,,}
        case "$reply" in
            "") [[ "$default" == "y" ]]; return ;;
            y | yes | t | tak) return 0 ;;
            n | no | nie) return 1 ;;
        esac
    done
}

# ask_value KEY default -> stdout
ask_value() {
    local reply
    if [[ "$ASSUME_YES" == "1" ]]; then printf '%s' "$2"; return; fi
    printf '%s?%s %s [%s]: ' "$C_GOLD" "$C_RESET" "$(t "$1")" "$2" >&2
    read_line
    reply=$REPLY_LINE
    printf '%s' "${reply:-$2}"
}

# --- error handling ----------------------------------------------------------------------
on_error() {
    local line=$1
    printf '\n' >&2
    printf '%s✖ %s%s\n' "$C_RED" "$(t error_at "$line" "$LOG_FILE")" "$C_RESET" >&2
}
trap 'on_error $LINENO' ERR

# Long commands: output to the log, a short status on screen.
run() {
    printf '%s  $ %s%s\n' "$C_DIM" "$*" "$C_RESET"
    { printf '\n$ %s\n' "$*"; } >>"$LOG_FILE"
    if ! "$@" >>"$LOG_FILE" 2>&1; then
        warn "$(tail -n 15 "$LOG_FILE")"
        return 1
    fi
}

# --- system detection --------------------------------------------------------------------
OS_ID="" OS_NAME="" PKG=""
OS_RELEASE=${LISEK_OS_RELEASE:-/etc/os-release}

detect_os() {
    [[ -r "$OS_RELEASE" ]] || fail "$(t os_unknown)"
    local like
    # shellcheck source=/dev/null
    OS_ID=$(. "$OS_RELEASE" && printf '%s' "${ID:-unknown}")
    # shellcheck source=/dev/null
    OS_NAME=$(. "$OS_RELEASE" && printf '%s' "${PRETTY_NAME:-${NAME:-$OS_ID}}")
    # shellcheck source=/dev/null
    like=$(. "$OS_RELEASE" && printf '%s' "${ID_LIKE:-}")
    case "$OS_ID" in
        debian | ubuntu) PKG=apt; ok "$(t os_ok "$OS_NAME")" ;;
        arch) PKG=pacman; ok "$(t os_ok "$OS_NAME")" ;;
        *)
            warn "$(t os_other "$OS_NAME")"
            ask_yes_no os_continue n || { info "$(t aborted)"; exit 0; }
            if [[ " $like " == *" debian "* || " $like " == *" ubuntu "* ]] || command -v apt-get >/dev/null; then
                PKG=apt
            elif [[ " $like " == *" arch "* ]] || command -v pacman >/dev/null; then
                PKG=pacman
            else
                fail "$(t os_no_pm)"
            fi
            ;;
    esac
}

APT_UPDATED=0
pkg_install() {
    [[ $# -gt 0 ]] || return 0
    info "$(t installing "$*")"
    if [[ "$PKG" == "apt" ]]; then
        if [[ "$APT_UPDATED" == "0" ]]; then run apt-get update; APT_UPDATED=1; fi
        run env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$@"
    else
        # No -Sy here: refreshing without upgrading is a partial upgrade, which Arch does not support.
        run pacman -S --needed --noconfirm "$@" || fail "$(t pacman_hint)"
    fi
}

have_systemd() { [[ -d /run/systemd/system ]] && command -v systemctl >/dev/null; }

arch_name() {
    case "$(uname -m)" in
        x86_64 | amd64) printf 'x86_64' ;;
        aarch64 | arm64) printf 'aarch64' ;;
        *) uname -m ;;
    esac
}

# --- common pieces ------------------------------------------------------------------------
base_tools() {
    if [[ "$PKG" == "apt" ]]; then
        pkg_install ca-certificates curl git tar gzip openssl
    else
        pkg_install ca-certificates curl git tar gzip openssl
    fi
}

clone_or_update() {
    install -d "$SRC_DIR"
    local repo dir
    for repo in "${REPOS[@]}"; do
        dir="$SRC_DIR/$repo"
        if [[ -d "$dir/.git" ]]; then
            info "$(t fetching "$repo")"
            run git -C "$dir" fetch --tags origin
            run git -C "$dir" checkout "$GIT_REF"
            # Tags have no upstream: only branches are pulled.
            if git -C "$dir" symbolic-ref -q HEAD >/dev/null; then run git -C "$dir" pull --ff-only; fi
        else
            info "$(t cloning "$repo")"
            run git clone --branch "$GIT_REF" "$GIT_BASE/$repo.git" "$dir"
        fi
    done
}

node_major() { node -p 'process.versions.node.split(".")[0]' 2>/dev/null || printf '0'; }

ensure_node() {
    step "$(t step_node)"
    local major
    major=$(command -v node >/dev/null && node_major || printf '0')
    if [[ "$major" -ge "$MIN_NODE_MAJOR" ]]; then ok "$(t node_found "$(node --version)")"; return; fi
    [[ "$major" == "0" ]] || warn "$(t node_old "$(node --version)" "$MIN_NODE_MAJOR" "$NODE_MAJOR")"
    if [[ "$PKG" == "apt" ]]; then
        # NodeSource: the distributions ship Node.js 18, too old for the web app.
        pkg_install ca-certificates curl gnupg
        local setup
        setup=$(mktemp)
        run curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" -o "$setup"
        run bash "$setup"
        rm -f "$setup"
        APT_UPDATED=0
        pkg_install nodejs
    else
        pkg_install nodejs npm
    fi
    ok "$(t node_found "$(node --version)")"
}

ensure_ffmpeg() {
    command -v ffmpeg >/dev/null && command -v ffprobe >/dev/null && return
    pkg_install ffmpeg
}

ensure_service_user() {
    if ! id "$SERVICE_USER" >/dev/null 2>&1; then
        local nologin
        nologin=$(command -v nologin || printf '/usr/sbin/nologin')
        useradd --system --home-dir "$DATA_DIR" --shell "$nologin" --user-group "$SERVICE_USER"
        ok "$(t user_created "$SERVICE_USER")"
    fi
    # GPU access for VA-API / Quick Sync / NVENC.
    local group
    for group in video render; do
        getent group "$group" >/dev/null && usermod -aG "$group" "$SERVICE_USER" || true
    done
}

# Sets KEY=value in an env file (only used on a .env created by this run).
set_env_var() {
    local file=$1 key=$2 value=$3
    if grep -q "^${key}=" "$file"; then
        local escaped=${value//\\/\\\\}
        escaped=${escaped//&/\\&}
        escaped=${escaped//|/\\|}
        sed -i "s|^${key}=.*|${key}=${escaped}|" "$file"
    else
        printf '%s=%s\n' "$key" "$value" >>"$file"
    fi
}

DB_URL=""
setup_postgres() {
    step "$(t step_db)"
    if ! ask_yes_no db_setup y; then info "$(t db_skip)"; return; fi
    if [[ "$PKG" == "apt" ]]; then
        pkg_install postgresql postgresql-contrib
    else
        pkg_install postgresql
        if [[ ! -s /var/lib/postgres/data/PG_VERSION ]]; then
            run su - postgres -c "initdb --locale=C.UTF-8 --encoding=UTF8 -D /var/lib/postgres/data"
        fi
    fi
    if have_systemd; then run systemctl enable --now postgresql; fi
    local _
    for _ in $(seq 1 30); do
        su - postgres -c "psql -tAc 'SELECT 1'" >/dev/null 2>&1 && break
        sleep 1
    done
    if [[ -n "$(su - postgres -c "psql -tAc \"SELECT 1 FROM pg_roles WHERE rolname='lisekmovie'\"")" ]]; then
        ok "$(t db_exists)"
        return
    fi
    local password
    password=$(openssl rand -hex 24)
    # SQL goes through stdin: the password never shows up in the console, the log or `ps`.
    printf '\n$ psql: CREATE ROLE lisekmovie / CREATE DATABASE lisekmovie / pg_trgm\n' >>"$LOG_FILE"
    su - postgres -c "psql -q -v ON_ERROR_STOP=1" >>"$LOG_FILE" 2>&1 <<SQL || fail "$(t db_failed "$LOG_FILE")"
CREATE ROLE lisekmovie LOGIN PASSWORD '${password}';
CREATE DATABASE lisekmovie OWNER lisekmovie ENCODING 'UTF8' TEMPLATE template0;
\\connect lisekmovie
CREATE EXTENSION IF NOT EXISTS pg_trgm;
SQL
    DB_URL="postgres://lisekmovie:${password}@127.0.0.1:5432/lisekmovie"
    ok "$(t db_created)"
}

# The server's .env lives in its working directory (the data folder): it
# is created once and never overwritten.
prepare_data_dir() {
    local example=$1
    install -d -m 0750 -o "$SERVICE_USER" -g "$SERVICE_USER" "$DATA_DIR" "$DATA_DIR/config"
    local env="$DATA_DIR/.env"
    if [[ -f "$env" ]]; then
        ok "$(t env_kept "$env")"
        [[ -z "$DB_URL" ]] || warn "DATABASE_URL: $(t env_kept "$env")"
        return
    fi
    install -m 0600 -o "$SERVICE_USER" -g "$SERVICE_USER" "$example" "$env"
    set_env_var "$env" LISEK_BIND_ADDR "127.0.0.1:8080"
    if [[ -n "$DB_URL" ]]; then
        set_env_var "$env" DATABASE_URL "$DB_URL"
    else
        # No database yet: without DATABASE_URL the first-run wizard asks for it.
        sed -i 's|^DATABASE_URL=|# DATABASE_URL=|' "$env"
    fi
    ok "$(t env_created "$env")"
}

# Replaces $APP_DIR with a staged build; the old one stays as app.prev.
deploy_app() {
    local staged=$1
    rm -rf "${APP_DIR}.prev"
    [[ -d "$APP_DIR" ]] && mv "$APP_DIR" "${APP_DIR}.prev"
    mv "$staged" "$APP_DIR"
    chown -R root:root "$APP_DIR"
}

write_services() {
    step "$(t step_service)"
    install -d /etc/lisekmovie
    # Web settings: written once, yours to edit afterwards.
    if [[ ! -f /etc/lisekmovie/web.env ]]; then
        cat >/etc/lisekmovie/web.env <<EOF
# lisekmovie-web (Node.js): where it listens and where lisek-server is.
PORT=${HTTP_PORT}
HOST=0.0.0.0
LISEK_API_URL=http://127.0.0.1:8080
# 1 behind another reverse proxy (Caddy, nginx, Traefik) with TLS.
TRUST_PROXY=0
EOF
    fi
    local node_bin
    node_bin=$(command -v node)
    if ! have_systemd; then
        warn "$(t systemd_missing)"
        printf '  cd %s && sudo -u %s %s/bin/lisek-server &\n' "$DATA_DIR" "$SERVICE_USER" "$APP_DIR"
        # shellcheck disable=SC2016 # printed literally for the user to run
        printf '  cd %s/web && sudo -u %s env $(grep -v "^#" /etc/lisekmovie/web.env | xargs) %s server.js &\n' "$APP_DIR" "$SERVICE_USER" "$node_bin"
        return
    fi
    cat >/etc/systemd/system/lisekmovie-server.service <<EOF
[Unit]
Description=LisekMovie server (API, scanner, transcoding)
After=network-online.target postgresql.service
Wants=network-online.target

[Service]
Type=simple
User=${SERVICE_USER}
Group=${SERVICE_USER}
SupplementaryGroups=video
WorkingDirectory=${DATA_DIR}
Environment=LISEK_CONFIG_DIR=${DATA_DIR}/config
Environment=RUST_LOG=info
ExecStart=${APP_DIR}/bin/lisek-server
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
ProtectSystem=full
PrivateTmp=true

[Install]
WantedBy=multi-user.target
EOF
    if getent group render >/dev/null; then
        sed -i 's/^SupplementaryGroups=video$/SupplementaryGroups=video render/' /etc/systemd/system/lisekmovie-server.service
    fi
    cat >/etc/systemd/system/lisekmovie-web.service <<EOF
[Unit]
Description=LisekMovie web app
After=network-online.target lisekmovie-server.service
Wants=lisekmovie-server.service

[Service]
Type=simple
User=${SERVICE_USER}
Group=${SERVICE_USER}
WorkingDirectory=${APP_DIR}/web
Environment=NODE_ENV=production
EnvironmentFile=/etc/lisekmovie/web.env
ExecStart=${node_bin} server.js
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
ProtectSystem=strict
PrivateTmp=true

[Install]
WantedBy=multi-user.target
EOF
    run systemctl daemon-reload
    run systemctl enable lisekmovie-server.service lisekmovie-web.service
    run systemctl restart lisekmovie-server.service lisekmovie-web.service
    ok "$(t service_started)"
}

wait_for_health() {
    info "$(t health_wait)"
    local _
    for _ in $(seq 1 60); do
        if curl -fsS --max-time 2 http://127.0.0.1:8080/api/health >/dev/null 2>&1; then ok "$(t health_ok)"; return 0; fi
        sleep 2
    done
    warn "$(t health_fail "journalctl -u lisekmovie-server -n 50")"
    return 1
}

write_conf() {
    local method=$1 version=$2
    install -d "$PREFIX"
    {
        printf '# Written by install-linux.sh; read by update-linux.sh.\n'
        printf 'METHOD=%q\n' "$method"
        printf 'PREFIX=%q\n' "$PREFIX"
        printf 'DATA_DIR=%q\n' "$DATA_DIR"
        printf 'SERVICE_USER=%q\n' "$SERVICE_USER"
        printf 'GIT_BASE=%q\n' "$GIT_BASE"
        printf 'GIT_REF=%q\n' "$GIT_REF"
        printf 'RELEASE_API=%q\n' "$RELEASE_API"
        printf 'LANG_CODE=%q\n' "$LANG_CODE"
        printf 'INSTALLED_VERSION=%q\n' "$version"
        printf 'INSTALLED_AT=%q\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >"$CONF_FILE"
    chmod 0644 "$CONF_FILE"
    # Keep the updater next to the installation.
    local here
    here=$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd || true)
    if [[ -f "$SRC_DIR/LisekMovie-app/update-linux.sh" ]]; then
        install -m 0755 "$SRC_DIR/LisekMovie-app/update-linux.sh" "$PREFIX/update-linux.sh"
    elif [[ -n "$here" && -f "$here/update-linux.sh" ]]; then
        install -m 0755 "$here/update-linux.sh" "$PREFIX/update-linux.sh"
    elif [[ -f "$APP_DIR/update-linux.sh" ]]; then
        install -m 0755 "$APP_DIR/update-linux.sh" "$PREFIX/update-linux.sh"
    fi
}

host_url() {
    local ip
    ip=$(hostname -I 2>/dev/null | awk '{print $1}')
    printf 'http://%s:%s' "${ip:-localhost}" "$1"
}

# --- 1) build from source -------------------------------------------------------------------
export RUSTUP_HOME="${PREFIX}/toolchain/rustup"
export CARGO_HOME="${PREFIX}/toolchain/cargo"

ensure_rust() {
    step "$(t step_rust)"
    if [[ ! -x "$CARGO_HOME/bin/cargo" ]]; then
        info "$(t rust_install "${PREFIX}/toolchain")"
        local rustup
        rustup=$(mktemp)
        run curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o "$rustup"
        run sh "$rustup" -y --profile minimal --default-toolchain stable --no-modify-path
        rm -f "$rustup"
    else
        run "$CARGO_HOME/bin/rustup" update stable
    fi
    export PATH="$CARGO_HOME/bin:$PATH"
    ok "$(t rust_found "$(rustc --version)")"
}

# Builds into a new folder (bin/, web/); prints its path.
build_from_source() {
    step "$(t step_build)"
    local backend="$SRC_DIR/LisekMovie-app/LisekMovie-Backend"
    local frontend="$SRC_DIR/LisekMovie-app/LisekMovie-Frontend"
    local staged="${APP_DIR}.new"
    rm -rf "$staged"
    info "$(t build_backend)"
    (
        cd "$backend"
        run env SQLX_OFFLINE=true CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=4 \
            CARGO_PROFILE_RELEASE_STRIP=symbols cargo build --release --locked -p lisek-server
    )
    info "$(t build_frontend)"
    (
        cd "$frontend"
        run npm ci --no-audit --no-fund
        run npm run build
    )
    install -d "$staged/bin" "$staged/web"
    install -m 0755 "$backend/target/release/lisek-server" "$staged/bin/lisek-server"
    cp -a "$frontend/build" "$frontend/server.js" "$frontend/package.json" "$frontend/package-lock.json" "$staged/web/"
    (cd "$staged/web" && run npm ci --omit=dev --ignore-scripts --no-audit --no-fund)
    local version
    version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$backend/Cargo.toml" | head -n1)
    printf '%s+%s\n' "$version" "$(git -C "$SRC_DIR/LisekMovie-app" rev-parse --short HEAD)" >"$staged/VERSION"
    cp "$SRC_DIR/LisekMovie-app/.env.example" "$staged/.env.example"
    ok "$(t build_done "$(cat "$staged/VERSION")")"
}

install_source() {
    step "$(t step_packages)"
    base_tools
    if [[ "$PKG" == "apt" ]]; then
        pkg_install build-essential pkg-config cmake perl
    else
        pkg_install base-devel pkgconf cmake perl
    fi
    ensure_ffmpeg
    step "$(t step_sources)"
    clone_or_update
    ensure_rust
    ensure_node
    build_from_source
    ensure_service_user
    setup_postgres
    prepare_data_dir "${APP_DIR}.new/.env.example"
    deploy_app "${APP_DIR}.new"
    write_conf source "$(cat "$APP_DIR/VERSION")"
    write_services
    have_systemd && wait_for_health || true
    finish "$(host_url "$HTTP_PORT")" "$DATA_DIR"
}

# --- 2) Docker Compose -------------------------------------------------------------------------
ensure_docker() {
    step "$(t step_docker)"
    if ! command -v docker >/dev/null || ! docker compose version >/dev/null 2>&1; then
        warn "$(t docker_install)"
        if [[ "$PKG" == "apt" ]]; then
            # Docker's own packages (docker-ce + compose plugin).
            local script
            script=$(mktemp)
            run curl -fsSL https://get.docker.com -o "$script"
            run sh "$script"
            rm -f "$script"
        else
            pkg_install docker docker-compose docker-buildx
        fi
        if have_systemd; then run systemctl enable --now docker; fi
    fi
    ok "$(t docker_found "$(docker version --format '{{.Client.Version}}' 2>/dev/null | head -n1)" "$(docker compose version --short 2>/dev/null || echo '?')")"
}

install_docker() {
    step "$(t step_packages)"
    base_tools
    ensure_docker
    step "$(t step_sources)"
    clone_or_update
    local app="$SRC_DIR/LisekMovie-app" env
    env="$app/.env"
    if [[ -f "$env" ]]; then
        ok "$(t env_kept "$env")"
        HTTP_PORT=$(sed -n 's/^LISEK_HTTP_PORT=//p' "$env" | tail -n1)
        HTTP_PORT=${HTTP_PORT:-8096}
    else
        cp "$app/.env.example" "$env"
        chmod 0600 "$env"
        local media files="docker-compose.yml"
        media=$(ask_value media_dir "/srv/media")
        install -d "$media"
        HTTP_PORT=$(ask_value http_port "$HTTP_PORT")
        set_env_var "$env" MEDIA_DIR "$media"
        set_env_var "$env" LISEK_HTTP_PORT "$HTTP_PORT"
        set_env_var "$env" POSTGRES_PASSWORD "$(openssl rand -hex 24)"
        set_env_var "$env" TZ "$(cat /etc/timezone 2>/dev/null || timedatectl show -p Timezone --value 2>/dev/null || echo UTC)"
        if command -v nvidia-smi >/dev/null && nvidia-smi -L >/dev/null 2>&1 && ask_yes_no docker_gpu_nvidia y; then
            files="$files:docker-compose.gpu.yml"
        fi
        if [[ -d /dev/dri ]] && ls /dev/dri/renderD* >/dev/null 2>&1 && ask_yes_no docker_gpu_intel y; then
            files="$files:docker-compose.intel.yml"
            set_env_var "$env" RENDER_GID "$(getent group render | cut -d: -f3)"
        fi
        # Compose reads COMPOSE_FILE from .env: updates use the same files.
        set_env_var "$env" COMPOSE_FILE "$files"
        ok "$(t env_created "$env")"
    fi
    info "$(t docker_up)"
    (cd "$app" && run docker compose up -d --build)
    ok "$(t docker_running)"
    write_conf docker "$(sed -n 's/^version = "\(.*\)"/\1/p' "$app/LisekMovie-Backend/Cargo.toml" | head -n1)+$(git -C "$app" rev-parse --short HEAD)"
    finish "$(host_url "$HTTP_PORT")" "docker volumes lisekmovie-postgres-data, lisekmovie-data · $env"
}

# --- 3) release binaries ------------------------------------------------------------------------
RELEASE_TAG="" RELEASE_URL="" RELEASE_SUM_URL=""

query_release() {
    info "$(t release_query)"
    local json arch
    arch=$(arch_name)
    json=$(curl -fsSL -H 'Accept: application/vnd.github+json' "$RELEASE_API") || fail "$(t release_none "$arch")"
    RELEASE_TAG=$(printf '%s' "$json" | grep -o '"tag_name": *"[^"]*"' | head -n1 | sed 's/.*"\([^"]*\)"$/\1/')
    RELEASE_URL=$(printf '%s' "$json" | grep -o '"browser_download_url": *"[^"]*linux-'"$arch"'\.tar\.gz"' | head -n1 | sed 's/.*"\(http[^"]*\)"$/\1/')
    RELEASE_SUM_URL=$(printf '%s' "$json" | grep -o '"browser_download_url": *"[^"]*linux-'"$arch"'\.tar\.gz\.sha256"' | head -n1 | sed 's/.*"\(http[^"]*\)"$/\1/')
    [[ -n "$RELEASE_URL" ]] || fail "$(t release_none "$arch")"
    ok "$(t release_found "$RELEASE_TAG")"
}

# Downloads and verifies the release into ${APP_DIR}.new.
fetch_release() {
    local tmp archive
    tmp=$(mktemp -d)
    archive="$tmp/${RELEASE_URL##*/}"
    info "$(t release_download "${RELEASE_URL##*/}")"
    run curl -fSL --retry 3 -o "$archive" "$RELEASE_URL"
    if [[ -n "$RELEASE_SUM_URL" ]]; then
        run curl -fsSL --retry 3 -o "$archive.sha256" "$RELEASE_SUM_URL"
        local expected actual
        expected=$(awk '{print $1}' "$archive.sha256")
        actual=$(sha256sum "$archive" | awk '{print $1}')
        if [[ "$expected" != "$actual" ]]; then rm -rf "$tmp"; fail "$(t checksum_bad)"; fi
        ok "$(t checksum_ok)"
    else
        warn "$(t checksum_missing)"
        ask_yes_no os_continue n || { rm -rf "$tmp"; info "$(t aborted)"; exit 0; }
    fi
    rm -rf "${APP_DIR}.new"
    install -d "${APP_DIR}.new"
    run tar -xzf "$archive" -C "${APP_DIR}.new" --strip-components=1 --no-same-owner
    rm -rf "$tmp"
    [[ -x "${APP_DIR}.new/bin/lisek-server" && -f "${APP_DIR}.new/web/server.js" ]] || fail "$(t checksum_bad)"
}

install_release() {
    step "$(t step_packages)"
    base_tools
    ensure_ffmpeg
    ensure_node
    step "$(t step_release)"
    query_release
    fetch_release
    ensure_service_user
    setup_postgres
    prepare_data_dir "${APP_DIR}.new/.env.example"
    deploy_app "${APP_DIR}.new"
    write_conf release "$RELEASE_TAG"
    write_services
    have_systemd && wait_for_health || true
    finish "$(host_url "$HTTP_PORT")" "$DATA_DIR"
}

finish() {
    local url=$1 data=$2
    printf '\n%s%s  %s%s\n\n' "$C_BOLD" "$C_GREEN" "$(t done_title)" "$C_RESET"
    printf '  %s\n' "$(t done_open "${C_BOLD}${url}${C_RESET}")"
    printf '  %s\n' "$(t done_data "$data")"
    printf '  %s\n' "$(t done_update "$PREFIX")"
    printf '  %s%s%s\n\n' "$C_DIM" "$(t done_log "$LOG_FILE")" "$C_RESET"
}

# --- main ----------------------------------------------------------------------------------------
choose_language() {
    case "$LANG_CODE" in en | pl) return ;; esac
    if [[ "$ASSUME_YES" == "1" ]]; then LANG_CODE=en; return; fi
    printf 'Select language / Wybierz język:\n  1) English\n  2) Polski\n'
    while true; do
        printf '%s?%s [1-2]: ' "$C_GOLD" "$C_RESET"
        read_line
        case "$REPLY_LINE" in
            1 | "") LANG_CODE=en; return ;;
            2) LANG_CODE=pl; return ;;
        esac
    done
}

choose_method() {
    case "$METHOD" in source | docker | release) return ;; esac
    printf '\n%s\n' "$(t method_title)"
    printf '  1) %s / %s\n' "${EN[method_1]}" "${PL[method_1]}"
    printf '  %s2) %s / %s%s\n' "$C_BOLD" "${EN[method_2]}" "${PL[method_2]}" "$C_RESET"
    printf '  3) %s / %s\n' "${EN[method_3]}" "${PL[method_3]}"
    if [[ "$ASSUME_YES" == "1" ]]; then METHOD=docker; return; fi
    while true; do
        printf '%s?%s %s [1-3]: ' "$C_GOLD" "$C_RESET" "$(t choice)"
        read_line
        case "$REPLY_LINE" in
            1) METHOD=source; return ;;
            2 | "") METHOD=docker; return ;;
            3) METHOD=release; return ;;
            *) warn "$(t invalid)" ;;
        esac
    done
}

main() {
    for arg in "$@"; do
        case "$arg" in
            --yes | -y) ASSUME_YES=1 ;;
            --lang=*) LANG_CODE=${arg#*=} ;;
            --method=*) METHOD=${arg#*=} ;;
            -h | --help) sed -n '2,32p' "${BASH_SOURCE[0]}"; exit 0 ;;
        esac
    done
    banner
    choose_language
    [[ $EUID -eq 0 ]] || fail "$(t need_root)"
    : >"$LOG_FILE"
    chmod 0600 "$LOG_FILE"
    detect_os
    if [[ -f "$CONF_FILE" ]]; then
        local old_method old_version
        old_method=$(sed -n 's/^METHOD=//p' "$CONF_FILE")
        old_version=$(sed -n 's/^INSTALLED_VERSION=//p' "$CONF_FILE")
        warn "$(t existing "$old_method" "$old_version" "$PREFIX")"
        ask_yes_no existing_continue n || { info "$(t aborted)"; exit 0; }
    fi
    choose_method
    case "$METHOD" in
        source) install_source ;;
        docker) install_docker ;;
        release) install_release ;;
    esac
}

main "$@"
