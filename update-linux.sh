#!/usr/bin/env bash
# =============================================================================
#  LisekMovie updater for Linux
# =============================================================================
#
#  Updates an installation made by install-linux.sh, with the same method:
#
#    source    git pull (app + plugins), release build, service restart
#    docker    git pull, docker compose pull, docker compose up -d --build
#    release   the latest GitHub Release package, service restart
#
#  Your data is never touched:
#    - .env is never written, replaced or deleted (checked with sha256
#      before and after the update);
#    - the PostgreSQL database is never dropped or reset: the new version
#      only applies its migrations, and a compressed backup (pg_dump) is
#      written to /var/backups/lisekmovie first;
#    - the data folder (posters, thumbnails, trickplay, cache) and the Docker
#      volumes are never removed; `docker compose down -v` is never run.
#  Only the program files are replaced. The previous version stays in
#  app.prev and comes back automatically when the new one does not start.
#
#  Usage:
#    sudo bash /opt/lisekmovie/update-linux.sh [options]
#
#  Options:
#    --check        only report whether an update is available
#                   (exit code 0: up to date, 10: update available)
#    --yes, -y      no questions
#    --force        rebuild / reinstall even when already up to date
#    --no-backup    skip the database backup
#    --lang=en|pl   language of the messages
#    -h, --help     this help
#
#  Environment: LISEK_PREFIX (default /opt/lisekmovie) when the installation
#  lives elsewhere, LISEK_BACKUP_DIR, LISEK_BACKUP_KEEP (default 5).
# =============================================================================

set -Eeuo pipefail
umask 022

# --- settings -----------------------------------------------------------------
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PREFIX="${LISEK_PREFIX:-/opt/lisekmovie}"
BACKUP_DIR="${LISEK_BACKUP_DIR:-/var/backups/lisekmovie}"
BACKUP_KEEP="${LISEK_BACKUP_KEEP:-5}"
LOG_FILE="${LISEK_LOG:-/tmp/lisekmovie-update.log}"
LOCK_FILE="${LISEK_LOCK:-/run/lock/lisekmovie-update.lock}"
REPOS=(LisekMovie-app LisekMovie-plugin-tmdb LisekMovie-plugin-thetvdb)
MIN_NODE_MAJOR=20
ASSUME_YES="${LISEK_ASSUME_YES:-0}"
CHECK_ONLY=0
FORCE=0
DO_BACKUP=1
LANG_ARG="${LISEK_LANG:-}"

# Filled from install.conf (all keys listed for clarity, not all are used here).
# shellcheck disable=SC2034
METHOD="" DATA_DIR="" SERVICE_USER="" GIT_BASE="" GIT_REF="" RELEASE_API=""
# shellcheck disable=SC2034
LANG_CODE="" INSTALLED_VERSION="" INSTALLED_AT=""
CONF_FILE="" SRC_DIR="" APP_DIR="" COMPOSE_DIR=""

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

# --- translations ------------------------------------------------------------------
declare -A EN=(
    [title]="LisekMovie updater"
    [need_root]="Run the updater as root: sudo bash %s"
    [no_install]="No LisekMovie installation was found (%s is missing). Install it with install-linux.sh, or set LISEK_PREFIX."
    [conf_unsafe]="%s must belong to root and must not be writable by other users."
    [detected_docker]="No install.conf: found a Docker Compose setup in %s."
    [method]="Installation: %s · version %s · %s"
    [locked]="Another update is already running."
    [step_check]="Checking for updates"
    [step_backup]="Database backup"
    [step_sources]="Source code"
    [step_build]="Build"
    [step_switch]="Switching to the new version"
    [step_docker]="Docker Compose"
    [step_release]="Release package"
    [dirty]="%s has local changes (git status). Commit or stash them first: the updater never overwrites your edits."
    [no_upstream]="%s is not on a branch (detached at %s): it is kept as it is."
    [behind]="%s: %s new commit(s)"
    [current]="%s: up to date"
    [up_to_date]="LisekMovie is up to date (%s)."
    [available]="An update is available: %s → %s"
    [forced]="--force: reinstalling anyway."
    [proceed]="Update now?"
    [aborted]="Update cancelled. Nothing was changed."
    [backup_skip]="--no-backup: skipping the database backup."
    [backup_ok]="Database backed up: %s (%s)"
    [backup_fail]="The database backup failed (details in %s)."
    [backup_continue]="Continue WITHOUT a database backup?"
    [backup_no_url]="DATABASE_URL is not set in %s: no backup."
    [backup_no_tool]="pg_dump was not found: no backup (install postgresql-client to get one)."
    [env_check]="%s unchanged (sha256 verified)."
    [env_changed]="%s was modified while the update ran (not by this script: the new server version may add generated settings when it starts). The content from before the update is saved in %s."
    [pull]="Updating %s"
    [build_backend]="Backend (cargo build --release): this takes a few minutes…"
    [build_frontend]="Web app (npm ci, npm run build)…"
    [build_done]="Built %s"
    [node_old]="Node.js %s is too old (need %s or newer)."
    [toolchain_missing]="The Rust toolchain from the installation is missing (%s). Run install-linux.sh again."
    [restart]="Restarting the services…"
    [health_wait]="Waiting for the server (migrations run on start)…"
    [health_ok]="The new version is running."
    [health_fail]="The new version did not start."
    [rollback]="Going back to the previous version (%s)…"
    [rollback_ok]="The previous version is running again. Nothing of your data was removed."
    [rollback_fail]="The previous version did not start either. Logs: journalctl -u lisekmovie-server -n 100"
    [rollback_db]="If the new version already changed the database, the old one may refuse it. Restore the backup: %s"
    [rollback_none]="There is no previous version to go back to."
    [no_systemd]="systemd was not found: restart lisek-server and the web app yourself."
    [compose_missing]="docker compose is not available."
    [compose_pull]="Pulling images (docker compose pull)…"
    [compose_up]="Rebuilding and restarting (docker compose up -d --build)…"
    [compose_ok]="Containers are running."
    [compose_fail]="The containers did not become healthy. Check: docker compose logs --tail=100 lisek-server"
    [pg_major]="The new compose file moves PostgreSQL from %s to %s. A major upgrade needs a dump and restore; the updater does not do it automatically. Your containers keep running the old version."
    [release_query]="Checking the latest release…"
    [release_none]="No release package was found for linux-%s."
    [release_download]="Downloading %s"
    [checksum_ok]="Checksum verified."
    [checksum_bad]="The checksum does not match: the download is damaged or was tampered with."
    [checksum_missing]="The release has no .sha256 file."
    [checksum_continue]="Install it without verification?"
    [package_bad]="The package does not contain bin/lisek-server and web/server.js."
    [self_update]="update-linux.sh updated."
    [done]="Update finished: %s"
    [done_safe]="Data kept: %s"
    [done_backup]="Database backup: %s"
    [done_log]="Full log: %s"
    [error_at]="Error on line %s. Nothing of your data was removed. Details: %s"
)
declare -A PL=(
    [title]="Aktualizacja LisekMovie"
    [need_root]="Uruchom aktualizację jako root: sudo bash %s"
    [no_install]="Nie znaleziono instalacji LisekMovie (brak %s). Zainstaluj ją przez install-linux.sh albo ustaw LISEK_PREFIX."
    [conf_unsafe]="%s musi należeć do roota i nie może być zapisywalny dla innych użytkowników."
    [detected_docker]="Brak install.conf: wykryto Docker Compose w %s."
    [method]="Instalacja: %s · wersja %s · %s"
    [locked]="Inna aktualizacja już trwa."
    [step_check]="Sprawdzanie aktualizacji"
    [step_backup]="Kopia bazy danych"
    [step_sources]="Kod źródłowy"
    [step_build]="Kompilacja"
    [step_switch]="Przełączanie na nową wersję"
    [step_docker]="Docker Compose"
    [step_release]="Paczka release"
    [dirty]="%s ma lokalne zmiany (git status). Najpierw je zacommituj albo schowaj (stash): aktualizacja nigdy nie nadpisuje Twoich zmian."
    [no_upstream]="%s nie jest na gałęzi (odłączony na %s): zostaje bez zmian."
    [behind]="%s: %s nowych commitów"
    [current]="%s: aktualne"
    [up_to_date]="LisekMovie jest aktualny (%s)."
    [available]="Dostępna aktualizacja: %s → %s"
    [forced]="--force: instaluję mimo to."
    [proceed]="Zaktualizować teraz?"
    [aborted]="Aktualizacja anulowana. Nic nie zostało zmienione."
    [backup_skip]="--no-backup: pomijam kopię bazy danych."
    [backup_ok]="Kopia bazy zapisana: %s (%s)"
    [backup_fail]="Kopia bazy danych nie powiodła się (szczegóły w %s)."
    [backup_continue]="Kontynuować BEZ kopii bazy danych?"
    [backup_no_url]="Brak DATABASE_URL w %s: kopia pominięta."
    [backup_no_tool]="Nie znaleziono pg_dump: kopia pominięta (zainstaluj postgresql-client)."
    [env_check]="%s bez zmian (sha256 zweryfikowane)."
    [env_changed]="%s zmienił się w trakcie aktualizacji (nie przez ten skrypt: nowa wersja serwera może dopisać wygenerowane ustawienia przy starcie). Treść sprzed aktualizacji zapisana w %s."
    [pull]="Aktualizuję %s"
    [build_backend]="Backend (cargo build --release): to potrwa kilka minut…"
    [build_frontend]="Aplikacja webowa (npm ci, npm run build)…"
    [build_done]="Zbudowano %s"
    [node_old]="Node.js %s jest za stary (wymagany %s lub nowszy)."
    [toolchain_missing]="Brak toolchaina Rust z instalacji (%s). Uruchom ponownie install-linux.sh."
    [restart]="Restartuję usługi…"
    [health_wait]="Czekam na serwer (migracje uruchamiają się przy starcie)…"
    [health_ok]="Nowa wersja działa."
    [health_fail]="Nowa wersja nie wystartowała."
    [rollback]="Wracam do poprzedniej wersji (%s)…"
    [rollback_ok]="Poprzednia wersja znowu działa. Żadne Twoje dane nie zostały usunięte."
    [rollback_fail]="Poprzednia wersja też nie wystartowała. Logi: journalctl -u lisekmovie-server -n 100"
    [rollback_db]="Jeśli nowa wersja zdążyła zmienić bazę, stara może jej nie przyjąć. Przywróć kopię: %s"
    [rollback_none]="Brak poprzedniej wersji do przywrócenia."
    [no_systemd]="Nie znaleziono systemd: zrestartuj lisek-server i aplikację webową samodzielnie."
    [compose_missing]="docker compose jest niedostępny."
    [compose_pull]="Pobieram obrazy (docker compose pull)…"
    [compose_up]="Przebudowuję i restartuję (docker compose up -d --build)…"
    [compose_ok]="Kontenery działają."
    [compose_fail]="Kontenery nie są zdrowe. Sprawdź: docker compose logs --tail=100 lisek-server"
    [pg_major]="Nowy plik compose zmienia PostgreSQL z %s na %s. Aktualizacja głównej wersji wymaga zrzutu i odtworzenia bazy; skrypt nie robi tego automatycznie. Kontenery dalej działają na starej wersji."
    [release_query]="Sprawdzam najnowszy release…"
    [release_none]="Nie znaleziono paczki dla linux-%s."
    [release_download]="Pobieram %s"
    [checksum_ok]="Suma kontrolna zgodna."
    [checksum_bad]="Suma kontrolna się nie zgadza: plik jest uszkodzony albo podmieniony."
    [checksum_missing]="Release nie ma pliku .sha256."
    [checksum_continue]="Zainstalować bez weryfikacji?"
    [package_bad]="Paczka nie zawiera bin/lisek-server i web/server.js."
    [self_update]="update-linux.sh zaktualizowany."
    [done]="Aktualizacja zakończona: %s"
    [done_safe]="Dane zachowane: %s"
    [done_backup]="Kopia bazy: %s"
    [done_log]="Pełny log: %s"
    [error_at]="Błąd w linii %s. Żadne Twoje dane nie zostały usunięte. Szczegóły: %s"
)

t() {
    local key=$1 fmt
    shift
    if [[ "$LANG_CODE" == "pl" ]]; then fmt=${PL[$key]:-${EN[$key]:-$key}}; else fmt=${EN[$key]:-$key}; fi
    # shellcheck disable=SC2059 # the translations are the format strings
    printf "$fmt" "$@"
}

# --- input ------------------------------------------------------------------------
TTY=/dev/tty
ask_yes_no() {
    local question default=$2 hint reply=""
    question=$(t "$1")
    if [[ "$ASSUME_YES" == "1" ]]; then [[ "$default" == "y" ]]; return; fi
    if [[ "$LANG_CODE" == "pl" ]]; then
        if [[ "$default" == "y" ]]; then hint="[T/n]"; else hint="[t/N]"; fi
    else
        if [[ "$default" == "y" ]]; then hint="[Y/n]"; else hint="[y/N]"; fi
    fi
    while true; do
        printf '%s?%s %s %s ' "$C_GOLD" "$C_RESET" "$question" "$hint"
        if [[ -r "$TTY" ]] && { : <"$TTY"; } 2>/dev/null; then
            IFS= read -r reply <"$TTY" || { printf '\n'; return 1; }
        else
            # No terminal: only the safe default ("no") is assumed.
            IFS= read -r reply || { printf '\n'; return 1; }
        fi
        reply=${reply,,}
        case "$reply" in
            "") [[ "$default" == "y" ]]; return ;;
            y | yes | t | tak) return 0 ;;
            n | no | nie) return 1 ;;
        esac
    done
}

# --- errors and logging ---------------------------------------------------------
# shellcheck disable=SC2329 # used by the trap
on_error() {
    printf '\n%s✖ %s%s\n' "$C_RED" "$(t error_at "$1" "$LOG_FILE")" "$C_RESET" >&2
}
trap 'on_error $LINENO' ERR

TMP_DIRS=()
# shellcheck disable=SC2329 # used by the trap
cleanup() {
    local dir
    for dir in "${TMP_DIRS[@]}"; do [[ -n "$dir" && "$dir" == /tmp/* ]] && rm -rf -- "$dir"; done
    return 0
}
trap cleanup EXIT

new_tmp() {
    local dir
    dir=$(mktemp -d /tmp/lisekmovie-update.XXXXXX)
    TMP_DIRS+=("$dir")
    printf '%s' "$dir"
}

# run CMD...: logs the command and its output; the last lines are shown on failure.
# The lock (fd 9) is not passed on: a service started from here must not hold it.
run() {
    printf '%s  $ %s%s\n' "$C_DIM" "$*" "$C_RESET"
    { printf '\n$ %s\n' "$*"; } >>"$LOG_FILE"
    if ! "$@" >>"$LOG_FILE" 2>&1 9>&-; then
        warn "$(tail -n 15 "$LOG_FILE")"
        return 1
    fi
}

# The only folders this script ever deletes: its own staging copies of the
# program. Anything else (data folder, .env, backups of the user) is refused.
remove_program_dir() {
    local dir=$1
    case "$dir" in
        "${APP_DIR}.new" | "${APP_DIR}.prev" | "${APP_DIR}.failed") ;;
        *) fail "refusing to remove $dir" ;;
    esac
    [[ -n "$APP_DIR" && "$APP_DIR" == "$PREFIX/app" ]] || fail "refusing to remove $dir"
    rm -rf -- "$dir"
}

# --- installation ---------------------------------------------------------------
load_install() {
    CONF_FILE="$PREFIX/install.conf"
    if [[ ! -f "$CONF_FILE" && -f "$SCRIPT_DIR/install.conf" ]]; then
        PREFIX=$SCRIPT_DIR
        CONF_FILE="$PREFIX/install.conf"
    fi
    if [[ -f "$CONF_FILE" ]]; then
        # Sourced as shell code, so it has to be as protected as this script.
        local owner mode
        owner=$(stat -c %u "$CONF_FILE")
        mode=$(stat -c %a "$CONF_FILE")
        if [[ "$owner" != "0" ]] || ((8#$mode & 8#022)); then fail "$(t conf_unsafe "$CONF_FILE")"; fi
        # shellcheck source=/dev/null
        . "$CONF_FILE"
        PREFIX=${PREFIX%/}
    elif [[ -f "$SCRIPT_DIR/docker-compose.yml" && -d "$SCRIPT_DIR/.git" ]]; then
        # A manual `git clone` + `docker compose up` (the README's quick start).
        METHOD=docker
        INSTALLED_VERSION=$(repo_git "$SCRIPT_DIR" rev-parse --short HEAD 2>/dev/null || printf '?')
        COMPOSE_DIR=$SCRIPT_DIR
        SRC_DIR=$(dirname "$SCRIPT_DIR")
        DETECTED=1
    else
        fail "$(t no_install "$PREFIX/install.conf")"
    fi
    SRC_DIR=${SRC_DIR:-$PREFIX/src}
    APP_DIR="$PREFIX/app"
    COMPOSE_DIR=${COMPOSE_DIR:-$SRC_DIR/LisekMovie-app}
    [[ -z "$LANG_ARG" ]] || LANG_CODE=$LANG_ARG
    if [[ "$LANG_CODE" != "en" && "$LANG_CODE" != "pl" ]]; then
        case "${LC_ALL:-${LANG:-}}" in pl*) LANG_CODE=pl ;; *) LANG_CODE=en ;; esac
    fi
    case "$METHOD" in source | docker | release) ;; *) fail "$(t no_install "$CONF_FILE")" ;; esac
}
DETECTED=0

set_conf() {
    local key=$1 value=$2
    [[ -f "$CONF_FILE" ]] || return 0
    local quoted
    quoted=$(printf '%q' "$value")
    if grep -q "^${key}=" "$CONF_FILE"; then
        local tmp
        tmp=$(mktemp "$CONF_FILE.XXXXXX")
        awk -v k="$key" -v v="$quoted" 'BEGIN { FS = OFS = "=" } $1 == k { print k "=" v; next } { print }' "$CONF_FILE" >"$tmp"
        chmod 0644 "$tmp"
        mv -f "$tmp" "$CONF_FILE"
    else
        printf '%s=%s\n' "$key" "$quoted" >>"$CONF_FILE"
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

# Reads one value from an env file without printing or sourcing it.
env_value() {
    local file=$1 key=$2 line
    line=$(grep -E "^[[:space:]]*${key}=" "$file" 2>/dev/null | tail -n1) || true
    line=${line#*=}
    line=${line%$'\r'}
    line=${line#\"}
    line=${line%\"}
    printf '%s' "$line"
}

# --- data safety: fingerprints of what must not change --------------------------------
PROTECTED_FILES=()
declare -A PROTECTED_SUMS=()
SAFE_COPY_DIR=""

protect_files() {
    local file
    SAFE_COPY_DIR=$(new_tmp)
    for file in "$@"; do
        [[ -f "$file" ]] || continue
        PROTECTED_FILES+=("$file")
        PROTECTED_SUMS[$file]=$(sha256sum "$file" | awk '{print $1}')
        cp -p -- "$file" "$SAFE_COPY_DIR/$(printf '%s' "$file" | tr '/' '_')"
    done
}

verify_protected() {
    local file now
    for file in "${PROTECTED_FILES[@]}"; do
        now=$(sha256sum "$file" 2>/dev/null | awk '{print $1}') || now=""
        if [[ "$now" == "${PROTECTED_SUMS[$file]}" ]]; then
            ok "$(t env_check "$file")"
        else
            # Never expected; keep the original next to the backups.
            install -d -m 0700 "$BACKUP_DIR"
            local saved
            saved="$BACKUP_DIR/$(basename "$file").before-update-$(date +%Y%m%d-%H%M%S)"
            cp -p -- "$SAFE_COPY_DIR/$(printf '%s' "$file" | tr '/' '_')" "$saved"
            warn "$(t env_changed "$file" "$saved")"
        fi
    done
}

# --- database backup ----------------------------------------------------------------
BACKUP_FILE=""

prune_backups() {
    # Only our own dump files, oldest first, beyond the newest $BACKUP_KEEP.
    local old
    while IFS= read -r old; do
        [[ "$old" == "$BACKUP_DIR"/lisekmovie-db-*.sql.gz ]] && rm -f -- "$old"
    done < <(find "$BACKUP_DIR" -maxdepth 1 -type f -name 'lisekmovie-db-*.sql.gz' -printf '%T@ %p\n' 2>/dev/null |
        sort -rn | awk -v keep="$BACKUP_KEEP" 'NR > keep { sub(/^[^ ]+ /, ""); print }')
}

finish_backup() {
    local tmp_file=$1
    if [[ -s "$tmp_file" ]] && gzip -t "$tmp_file" 2>/dev/null; then
        mv -f "$tmp_file" "$BACKUP_FILE"
        ok "$(t backup_ok "$BACKUP_FILE" "$(du -h "$BACKUP_FILE" | awk '{print $1}')")"
        prune_backups
        return 0
    fi
    rm -f -- "$tmp_file"
    BACKUP_FILE=""
    return 1
}

backup_failed() {
    warn "$(t backup_fail "$LOG_FILE")"
    ask_yes_no backup_continue n || { info "$(t aborted)"; exit 1; }
}

# postgres://user:password@host:port/db?params -> libpq environment (the
# password stays out of the process list).
pg_env_from_url() {
    local url=$1 rest auth hostport
    rest=${url#*://}
    rest=${rest%%\?*}
    auth=${rest%@*}
    if [[ "$auth" == "$rest" ]]; then auth=""; fi
    hostport=${rest##*@}
    PG_DB=${hostport#*/}
    hostport=${hostport%%/*}
    PG_USER=${auth%%:*}
    PG_PASS=""
    if [[ "$auth" == *:* ]]; then PG_PASS=${auth#*:}; fi
    # URL decoding (%40 -> @).
    PG_PASS=$(printf '%b' "${PG_PASS//%/\\x}")
    PG_USER=$(printf '%b' "${PG_USER//%/\\x}")
    PG_HOST=${hostport%%:*}
    PG_PORT=5432
    if [[ "$hostport" == *:* ]]; then PG_PORT=${hostport##*:}; fi
}

backup_native() {
    step "$(t step_backup)"
    if [[ "$DO_BACKUP" == "0" ]]; then warn "$(t backup_skip)"; return; fi
    local env="$DATA_DIR/.env" url
    url=$(env_value "$env" DATABASE_URL)
    if [[ -z "$url" ]]; then warn "$(t backup_no_url "$env")"; return; fi
    if ! command -v pg_dump >/dev/null; then warn "$(t backup_no_tool)"; backup_failed; return; fi
    local PG_DB PG_USER PG_PASS PG_HOST PG_PORT
    pg_env_from_url "$url"
    install -d -m 0700 "$BACKUP_DIR"
    BACKUP_FILE="$BACKUP_DIR/lisekmovie-db-$(date +%Y%m%d-%H%M%S)-${INSTALLED_VERSION//[^A-Za-z0-9._+-]/_}.sql.gz"
    local tmp_file="$BACKUP_FILE.partial"
    printf '\n$ pg_dump %s@%s:%s/%s | gzip\n' "$PG_USER" "$PG_HOST" "$PG_PORT" "$PG_DB" >>"$LOG_FILE"
    if (umask 077 && PGPASSWORD=$PG_PASS pg_dump --no-owner --no-privileges -h "$PG_HOST" -p "$PG_PORT" \
        -U "$PG_USER" -d "$PG_DB" 2>>"$LOG_FILE" | gzip -c >"$tmp_file") && finish_backup "$tmp_file"; then
        return
    fi
    rm -f -- "$tmp_file"
    BACKUP_FILE=""
    backup_failed
}

backup_docker() {
    step "$(t step_backup)"
    if [[ "$DO_BACKUP" == "0" ]]; then warn "$(t backup_skip)"; return; fi
    if [[ -z "$(cd "$COMPOSE_DIR" && docker compose ps -q postgres 2>/dev/null)" ]]; then
        warn "postgres: not running"
        backup_failed
        return
    fi
    install -d -m 0700 "$BACKUP_DIR"
    BACKUP_FILE="$BACKUP_DIR/lisekmovie-db-$(date +%Y%m%d-%H%M%S)-${INSTALLED_VERSION//[^A-Za-z0-9._+-]/_}.sql.gz"
    local tmp_file="$BACKUP_FILE.partial"
    printf '\n$ docker compose exec -T postgres pg_dump -U lisekmovie lisekmovie | gzip\n' >>"$LOG_FILE"
    if (cd "$COMPOSE_DIR" && umask 077 && docker compose exec -T postgres \
        pg_dump --no-owner --no-privileges -U lisekmovie lisekmovie 2>>"$LOG_FILE" | gzip -c >"$tmp_file") &&
        finish_backup "$tmp_file"; then
        return
    fi
    rm -f -- "$tmp_file"
    BACKUP_FILE=""
    backup_failed
}

# --- git ----------------------------------------------------------------------------------
# git as the owner of the checkout: a clone made by a normal user stays theirs
# (no root-owned files in it, no "dubious ownership" refusal).
repo_git() {
    local dir=$1 owner
    shift
    owner=$(stat -c %U "$dir")
    if [[ "$owner" == "root" || "$EUID" -ne 0 ]]; then
        git -C "$dir" "$@"
    else
        runuser -u "$owner" -- git -C "$dir" "$@"
    fi
}

# Sets UPDATES (number of new commits over all repositories).
UPDATES=0
check_git() {
    step "$(t step_check)"
    local repo dir behind
    UPDATES=0
    for repo in "${REPOS[@]}"; do
        dir="$SRC_DIR/$repo"
        [[ -d "$dir/.git" ]] || fail "$(t no_install "$dir")"
        # Tracked files only: .env, media and build output are ignored by git.
        if [[ -n "$(repo_git "$dir" status --porcelain --untracked-files=no)" ]]; then
            fail "$(t dirty "$dir")"
        fi
        run repo_git "$dir" fetch --tags --prune origin
        if ! repo_git "$dir" symbolic-ref -q HEAD >/dev/null; then
            warn "$(t no_upstream "$repo" "$(repo_git "$dir" rev-parse --short HEAD)")"
            continue
        fi
        behind=$(repo_git "$dir" rev-list --count "HEAD..@{upstream}" 2>/dev/null || printf '0')
        if [[ "$behind" -gt 0 ]]; then
            info "$(t behind "$repo" "$behind")"
            UPDATES=$((UPDATES + behind))
        else
            ok "$(t current "$repo")"
        fi
    done
}

pull_git() {
    step "$(t step_sources)"
    local repo dir
    for repo in "${REPOS[@]}"; do
        dir="$SRC_DIR/$repo"
        repo_git "$dir" symbolic-ref -q HEAD >/dev/null || continue
        info "$(t pull "$repo")"
        # Fast-forward only: never a merge, never a reset of local work.
        run repo_git "$dir" pull --ff-only
    done
}

source_version() {
    local app="$SRC_DIR/LisekMovie-app"
    printf '%s+%s' "$(sed -n 's/^version = "\(.*\)"/\1/p' "$app/LisekMovie-Backend/Cargo.toml" | head -n1)" \
        "$(repo_git "$app" rev-parse --short HEAD)"
}

confirm_update() {
    local from=$1 to=$2
    if [[ "$CHECK_ONLY" == "1" ]]; then
        if [[ "$UPDATES" -gt 0 ]]; then info "$(t available "$from" "$to")"; exit 10; fi
        ok "$(t up_to_date "$from")"
        exit 0
    fi
    if [[ "$UPDATES" -eq 0 ]]; then
        if [[ "$FORCE" == "0" ]]; then ok "$(t up_to_date "$from")"; exit 0; fi
        warn "$(t forced)"
    else
        info "$(t available "$from" "$to")"
    fi
    ask_yes_no proceed y || { info "$(t aborted)"; exit 0; }
}

# --- services and switching (source / release) ---------------------------------------------
health_url() {
    local bind
    bind=$(env_value "$DATA_DIR/.env" LISEK_BIND_ADDR)
    bind=${bind:-127.0.0.1:8080}
    bind=${bind/0.0.0.0/127.0.0.1}
    printf 'http://%s/api/health' "$bind"
}

wait_for_health() {
    local url _
    url=$(health_url)
    info "$(t health_wait)"
    local good=0
    for _ in $(seq 1 90); do
        # Three answers in a row, with the unit still up: a binary that crashes
        # right after start is "activating (auto-restart)", not "active".
        if curl -fsS --max-time 2 "$url" >/dev/null 2>&1 &&
            { ! have_systemd || systemctl is-active --quiet lisekmovie-server.service; }; then
            good=$((good + 1))
            [[ "$good" -lt 3 ]] || return 0
        else
            good=0
        fi
        sleep 2
    done
    return 1
}

restart_services() {
    info "$(t restart)"
    run systemctl restart lisekmovie-server.service
    run systemctl restart lisekmovie-web.service
}

# app.new -> app (the running version moves to app.prev), restart, health
# check, automatic rollback.
switch_version() {
    step "$(t step_switch)"
    local staged="${APP_DIR}.new"
    [[ -x "$staged/bin/lisek-server" && -f "$staged/web/server.js" ]] || fail "$(t package_bad)"
    chown -R root:root "$staged"
    [[ -d "${APP_DIR}.prev" ]] && remove_program_dir "${APP_DIR}.prev"
    if [[ -d "$APP_DIR" ]]; then mv -- "$APP_DIR" "${APP_DIR}.prev"; fi
    mv -- "$staged" "$APP_DIR"
    if ! have_systemd; then
        warn "$(t no_systemd)"
        return 0
    fi
    restart_services
    if wait_for_health; then
        ok "$(t health_ok)"
        return 0
    fi
    warn "$(t health_fail)"
    if [[ ! -d "${APP_DIR}.prev" ]]; then
        warn "$(t rollback_none)"
        fail "$(t health_fail)"
    fi
    warn "$(t rollback "$(cat "${APP_DIR}.prev/VERSION" 2>/dev/null || printf '?')")"
    [[ -d "${APP_DIR}.failed" ]] && remove_program_dir "${APP_DIR}.failed"
    mv -- "$APP_DIR" "${APP_DIR}.failed"
    mv -- "${APP_DIR}.prev" "$APP_DIR"
    restart_services || true
    if wait_for_health; then
        ok "$(t rollback_ok)"
    else
        warn "$(t rollback_fail)"
        [[ -z "$BACKUP_FILE" ]] || warn "$(t rollback_db "$BACKUP_FILE")"
    fi
    exit 1
}

check_node() {
    local major
    major=$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || printf '0')
    [[ "$major" -ge "$MIN_NODE_MAJOR" ]] || fail "$(t node_old "$(node --version 2>/dev/null || printf '-')" "$MIN_NODE_MAJOR")"
}

# Installs the updater shipped with the new version next to the installation.
# Written to a temporary file and renamed: the running copy is never modified.
self_update() {
    local candidate=$1
    [[ -f "$candidate" && -f "$CONF_FILE" ]] || return 0
    [[ -f "$PREFIX/update-linux.sh" ]] && cmp -s "$candidate" "$PREFIX/update-linux.sh" && return 0
    local tmp
    tmp=$(mktemp "$PREFIX/.update-linux.sh.XXXXXX")
    cat -- "$candidate" >"$tmp"
    chmod 0755 "$tmp"
    mv -f -- "$tmp" "$PREFIX/update-linux.sh"
    ok "$(t self_update)"
}

# --- 1) source --------------------------------------------------------------------------
build_from_source() {
    step "$(t step_build)"
    export RUSTUP_HOME="$PREFIX/toolchain/rustup" CARGO_HOME="$PREFIX/toolchain/cargo"
    [[ -x "$CARGO_HOME/bin/cargo" ]] || fail "$(t toolchain_missing "$CARGO_HOME")"
    export PATH="$CARGO_HOME/bin:$PATH"
    # New versions may need a newer compiler (rust-version in Cargo.toml).
    run rustup update stable
    check_node
    local backend="$SRC_DIR/LisekMovie-app/LisekMovie-Backend"
    local frontend="$SRC_DIR/LisekMovie-app/LisekMovie-Frontend"
    local staged="${APP_DIR}.new"
    [[ -d "$staged" ]] && remove_program_dir "$staged"
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
    source_version >"$staged/VERSION"
    printf '\n' >>"$staged/VERSION"
    cp "$SRC_DIR/LisekMovie-app/.env.example" "$staged/.env.example"
    ok "$(t build_done "$(cat "$staged/VERSION")")"
}

update_source() {
    check_git
    confirm_update "$INSTALLED_VERSION" "$(repo_git "$SRC_DIR/LisekMovie-app" rev-parse --short '@{upstream}' 2>/dev/null || printf '?')"
    protect_files "$DATA_DIR/.env" /etc/lisekmovie/web.env
    pull_git
    build_from_source
    backup_native
    switch_version
    set_conf INSTALLED_VERSION "$(cat "$APP_DIR/VERSION")"
    set_conf UPDATED_AT "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    self_update "$SRC_DIR/LisekMovie-app/update-linux.sh"
    verify_protected
    finish "$(cat "$APP_DIR/VERSION")" "$DATA_DIR"
}

# --- 2) docker ------------------------------------------------------------------------------
pg_image_major() {
    # postgres:18-alpine -> 18
    local image=$1 tag
    tag=${image##*:}
    [[ "$image" == *:* ]] || tag=""
    printf '%s' "${tag%%[!0-9]*}"
}

update_docker() {
    docker compose version >/dev/null 2>&1 || fail "$(t compose_missing)"
    check_git
    confirm_update "$INSTALLED_VERSION" "$(repo_git "$SRC_DIR/LisekMovie-app" rev-parse --short '@{upstream}' 2>/dev/null || printf '?')"
    protect_files "$COMPOSE_DIR/.env"
    pull_git
    step "$(t step_docker)"
    cd "$COMPOSE_DIR"
    # A new major PostgreSQL version cannot read the old data folder: stop here
    # instead of letting the container fail on start.
    local running_image new_image old_major new_major container
    container=$(docker compose ps -q postgres 2>/dev/null || true)
    running_image=""
    [[ -z "$container" ]] || running_image=$(docker inspect -f '{{.Config.Image}}' "$container" 2>/dev/null || true)
    new_image=$(docker compose config --images 2>/dev/null | grep -m1 '^postgres' || true)
    old_major=$(pg_image_major "$running_image")
    new_major=$(pg_image_major "$new_image")
    if [[ -n "$old_major" && -n "$new_major" && "$old_major" != "$new_major" ]]; then
        fail "$(t pg_major "$running_image" "$new_image")"
    fi
    backup_docker
    info "$(t compose_pull)"
    # Images built here (lisek-server, lisek-web) are rebuilt below, not pulled.
    if docker compose pull --help 2>/dev/null | grep -q -- '--ignore-buildable'; then
        run docker compose pull --ignore-buildable
    else
        run docker compose pull --ignore-pull-failures
    fi
    info "$(t compose_up)"
    # Volumes (database, posters, thumbnails) are kept: no `down`, no `-v`.
    run docker compose up -d --build
    local port _ healthy=0
    port=$(env_value "$COMPOSE_DIR/.env" LISEK_HTTP_PORT)
    port=${port:-8096}
    info "$(t health_wait)"
    for _ in $(seq 1 90); do
        if curl -fsS --max-time 2 "http://127.0.0.1:${port}/api/health" >/dev/null 2>&1; then healthy=1; break; fi
        sleep 2
    done
    if [[ "$healthy" == "1" ]]; then ok "$(t compose_ok)"; else
        warn "$(t compose_fail)"
        [[ -z "$BACKUP_FILE" ]] || warn "$(t rollback_db "$BACKUP_FILE")"
        exit 1
    fi
    local version
    version=$(source_version)
    set_conf INSTALLED_VERSION "$version"
    set_conf UPDATED_AT "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    [[ "$DETECTED" == "1" ]] || self_update "$COMPOSE_DIR/update-linux.sh"
    verify_protected
    finish "$version" "docker volumes lisekmovie-postgres-data, lisekmovie-data · $COMPOSE_DIR/.env"
}

# --- 3) release -----------------------------------------------------------------------------
RELEASE_TAG="" RELEASE_URL="" RELEASE_SUM_URL=""

query_release() {
    step "$(t step_check)"
    info "$(t release_query)"
    local json arch
    arch=$(arch_name)
    json=$(curl -fsSL -H 'Accept: application/vnd.github+json' "$RELEASE_API") || fail "$(t release_none "$arch")"
    RELEASE_TAG=$(printf '%s' "$json" | grep -o '"tag_name": *"[^"]*"' | head -n1 | sed 's/.*"\([^"]*\)"$/\1/')
    RELEASE_URL=$(printf '%s' "$json" | grep -o '"browser_download_url": *"[^"]*linux-'"$arch"'\.tar\.gz"' | head -n1 | sed 's/.*"\(http[^"]*\)"$/\1/')
    RELEASE_SUM_URL=$(printf '%s' "$json" | grep -o '"browser_download_url": *"[^"]*linux-'"$arch"'\.tar\.gz\.sha256"' | head -n1 | sed 's/.*"\(http[^"]*\)"$/\1/')
    [[ -n "$RELEASE_TAG" && -n "$RELEASE_URL" ]] || fail "$(t release_none "$arch")"
}

fetch_release() {
    step "$(t step_release)"
    local tmp archive
    tmp=$(new_tmp)
    archive="$tmp/${RELEASE_URL##*/}"
    info "$(t release_download "${RELEASE_URL##*/}")"
    run curl -fSL --retry 3 -o "$archive" "$RELEASE_URL"
    if [[ -n "$RELEASE_SUM_URL" ]]; then
        run curl -fsSL --retry 3 -o "$archive.sha256" "$RELEASE_SUM_URL"
        local expected actual
        expected=$(awk '{print $1}' "$archive.sha256")
        actual=$(sha256sum "$archive" | awk '{print $1}')
        [[ "$expected" == "$actual" ]] || fail "$(t checksum_bad)"
        ok "$(t checksum_ok)"
    else
        warn "$(t checksum_missing)"
        ask_yes_no checksum_continue n || { info "$(t aborted)"; exit 1; }
    fi
    local staged="${APP_DIR}.new"
    [[ -d "$staged" ]] && remove_program_dir "$staged"
    install -d "$staged"
    run tar -xzf "$archive" -C "$staged" --strip-components=1 --no-same-owner
    [[ -x "$staged/bin/lisek-server" && -f "$staged/web/server.js" ]] || fail "$(t package_bad)"
}

update_release() {
    query_release
    UPDATES=1
    [[ "$RELEASE_TAG" != "$INSTALLED_VERSION" ]] || UPDATES=0
    confirm_update "$INSTALLED_VERSION" "$RELEASE_TAG"
    check_node
    protect_files "$DATA_DIR/.env" /etc/lisekmovie/web.env
    fetch_release
    backup_native
    switch_version
    set_conf INSTALLED_VERSION "$RELEASE_TAG"
    set_conf UPDATED_AT "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    self_update "$APP_DIR/update-linux.sh"
    verify_protected
    finish "$RELEASE_TAG" "$DATA_DIR"
}

# --- main ----------------------------------------------------------------------------------------
finish() {
    local version=$1 data=$2
    printf '\n%s%s  %s%s\n\n' "$C_BOLD" "$C_GREEN" "$(t "done" "$version")" "$C_RESET"
    printf '  %s\n' "$(t done_safe "$data")"
    [[ -z "$BACKUP_FILE" ]] || printf '  %s\n' "$(t done_backup "$BACKUP_FILE")"
    printf '  %s%s%s\n\n' "$C_DIM" "$(t done_log "$LOG_FILE")" "$C_RESET"
}

main() {
    local arg
    for arg in "$@"; do
        case "$arg" in
            --check) CHECK_ONLY=1 ;;
            --yes | -y) ASSUME_YES=1 ;;
            --force) FORCE=1 ;;
            --no-backup) DO_BACKUP=0 ;;
            --lang=*) LANG_ARG=${arg#*=} ;;
            -h | --help) sed -n '2,38p' "${BASH_SOURCE[0]}"; exit 0 ;;
            *) printf 'Unknown option: %s (see --help)\n' "$arg" >&2; exit 2 ;;
        esac
    done
    LANG_CODE=$LANG_ARG
    [[ $EUID -eq 0 ]] || fail "$(t need_root "${BASH_SOURCE[0]}")"
    load_install
    printf '%s%s%s %s\n' "$C_BOLD" "$C_GOLD" "$(t title)" "$C_RESET"
    [[ "$DETECTED" == "0" ]] || info "$(t detected_docker "$COMPOSE_DIR")"
    info "$(t method "$METHOD" "${INSTALLED_VERSION:-?}" "$PREFIX")"

    : >"$LOG_FILE"
    chmod 0600 "$LOG_FILE"
    install -d "$(dirname "$LOCK_FILE")"
    exec 9>"$LOCK_FILE"
    flock -n 9 || fail "$(t locked)"

    case "$METHOD" in
        source) update_source ;;
        docker) update_docker ;;
        release) update_release ;;
    esac
}

# Braces: bash reads the whole script before running it, so replacing this
# file during the update (self_update) cannot affect the running copy.
{
    main "$@"
    exit $?
}
