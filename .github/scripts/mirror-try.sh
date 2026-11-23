# Sourced (sh or bash) inside a clean distro container before it installs
# packages:
#
#   . /scripts/mirror-try.sh
#   mirror_try apt-get update -qq
#
# Ubuntu's own package servers are sometimes so slow that an install taking
# seconds takes 10 to 40 minutes. mirror_try gives the command MIRROR_LIMIT
# seconds (180: a normal run takes under a minute), then switches the system to
# Azure's copy of Ubuntu's servers and runs it again, with a warning in the run.
# It stays switched, so later commands use Azure from the start.
# Not Ubuntu or Debian (no /etc/apt): the command just runs.
mirror_try() {
  [ -d /etc/apt ] || { "$@"; return; }
  _limit="${MIRROR_LIMIT:-180}"
  _rc=0
  timeout -k 10 "$_limit" "$@" || _rc=$?
  case "$_rc" in
    124 | 137) ;;
    *) return "$_rc" ;;
  esac
  echo "::warning::Ubuntu's package servers took over $_limit s, trying again with Azure's Ubuntu mirror"
  grep -rlE --include='*.list' --include='*.sources' 'http://(archive|security)\.ubuntu\.com' /etc/apt \
    | xargs -r sed -i -E 's#http://(archive|security)\.ubuntu\.com#http://azure.archive.ubuntu.com#'
  "$@"
}
