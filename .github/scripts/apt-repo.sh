#!/usr/bin/env bash
# The APT repository (v0.9.0): adds a release's .deb to a checkout of the
# archtoys-apt repo (GitHub Pages serves it at https://mujtaba1i.github.io/archtoys-apt),
# keeps the newest few versions, rebuilds the package index and signs it.
#
#   apt-repo.sh DEB REPO_DIR
#
# The signing key must already be in gpg's keyring (release.yml imports the
# APT_GPG_PRIVATE_KEY secret). APT_GPG_PASSPHRASE is used when the key has one.
# Needs apt-ftparchive (apt-utils) and gpg.
set -euo pipefail

deb=$1
repo=$2
keep=${APT_KEEP:-3}
url=${APT_URL:-https://mujtaba1i.github.io/archtoys-apt}

[ -f "$deb" ] || {
  echo "no .deb at $deb" >&2
  exit 1
}

# Resolve package path before changing directories.
deb=$(realpath "$deb")
key=$(gpg --batch --list-secret-keys --with-colons | awk -F: '$1 == "fpr" { print $10; exit }')
[ -n "$key" ] || { echo "no signing key in gpg's keyring (the APT_GPG_PRIVATE_KEY secret)" >&2; exit 1; }
sign=(gpg --batch --yes --pinentry-mode loopback --local-user "$key")
if [ -n "${APT_GPG_PASSPHRASE:-}" ]; then
  sign+=(--passphrase "$APT_GPG_PASSPHRASE")
fi

pool=pool/main/a/archtoys
mkdir -p "$repo/$pool" "$repo/dists/stable/main/binary-amd64"
cp "$deb" "$repo/$pool/"
cd "$repo"
# the newest $keep versions stay, so `apt install archtoys=OLD` works for a while
ls "$pool"/archtoys_*_amd64.deb | sort -V | head -n -"$keep" | xargs -r rm -f

apt-ftparchive --arch amd64 packages "$pool" > dists/stable/main/binary-amd64/Packages
gzip -9nkf dists/stable/main/binary-amd64/Packages
# The index of dists/stable must not list the previous run's Release files
# (it did: every run since the first listed the old Release in the new one)
rm -f dists/stable/Release dists/stable/InRelease dists/stable/Release.gpg
release_tmp=$(mktemp)
apt-ftparchive \
  -o APT::FTPArchive::Release::Origin=Archtoys \
  -o APT::FTPArchive::Release::Label=Archtoys \
  -o APT::FTPArchive::Release::Suite=stable \
  -o APT::FTPArchive::Release::Codename=stable \
  -o APT::FTPArchive::Release::Architectures=amd64 \
  -o APT::FTPArchive::Release::Components=main \
  -o "APT::FTPArchive::Release::Description=Archtoys, PowerToys-style tools for Linux" \
  release dists/stable > "$release_tmp"
mv "$release_tmp" dists/stable/Release
chmod 644 dists/stable/Release
"${sign[@]}" --clearsign -o dists/stable/InRelease dists/stable/Release
"${sign[@]}" --armor --detach-sign -o dists/stable/Release.gpg dists/stable/Release

# the public key: binary for apt's signed-by, armored to read
gpg --batch --yes --export "$key" > archtoys.gpg
gpg --batch --yes --armor --export "$key" > archtoys.asc
# Pages serves the files as they are (no Jekyll)
touch .nojekyll

version=$(dpkg-deb -f "$deb" Version)
cat > index.html <<EOF
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Archtoys APT repository</title>
<style>body{font:16px/1.6 system-ui,sans-serif;max-width:760px;margin:40px auto;padding:0 16px;color:#16181d}
pre{background:#f1f2f5;padding:14px;border-radius:10px;overflow-x:auto}code{font-size:14px}
@media (prefers-color-scheme:dark){body{background:#111216;color:#f2f3f6}pre{background:#1d1f25}a{color:#8fd0fa}}</style></head>
<body><h1>Archtoys APT repository</h1>
<p>For Debian 12+, Ubuntu 22.04+ and their families (amd64). Newest version: <b>$version</b>.
<a href="https://github.com/Mujtaba1i/Archtoys">Archtoys on GitHub</a>.</p>
<h2>Add it and install</h2>
<pre><code>curl -fsSL $url/archtoys.gpg | sudo tee /usr/share/keyrings/archtoys.gpg &gt; /dev/null
echo "deb [signed-by=/usr/share/keyrings/archtoys.gpg] $url stable main" | sudo tee /etc/apt/sources.list.d/archtoys.list
sudo apt update
sudo apt install archtoys</code></pre>
<p>Updates then come with the system's own updates (or <code>sudo apt upgrade</code>), and Archtoys' own update button uses apt too.</p>
<h2>Remove it</h2>
<pre><code>sudo apt remove archtoys
sudo rm /etc/apt/sources.list.d/archtoys.list /usr/share/keyrings/archtoys.gpg</code></pre>
</body></html>
EOF
echo "APT repository: archtoys $version added, $(ls "$pool" | wc -l) version(s) kept, signed with $key"
