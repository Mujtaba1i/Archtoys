pkgname=archtoys
pkgver=0.1.4
pkgrel=1
pkgdesc="PowerToys-like color picker for Linux (Slint-based)"
arch=('x86_64')
url="https://github.com/Mujtaba1i/Archtoys"
license=('MIT')
depends=('glibc')
makedepends=('cargo' 'git')
install=archtoys.install
source=("$pkgname-$pkgver.tar.gz::https://github.com/Mujtaba1i/Archtoys/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
  cd "Archtoys-$pkgver"
  cargo build --release --locked
}

package() {
  cd "Archtoys-$pkgver"

  install -Dm755 target/release/color-picker "$pkgdir/usr/lib/archtoys/archtoys-bin"
  install -Dm755 /dev/stdin "$pkgdir/usr/bin/archtoys" <<'WRAP'
#!/bin/sh
export SLINT_BACKEND=winit
exec /usr/lib/archtoys/archtoys-bin "$@"
WRAP

  if [[ -f packaging/archtoys.desktop ]]; then
    install -Dm644 packaging/archtoys.desktop "$pkgdir/usr/share/applications/archtoys.desktop"
  else
    install -Dm644 /dev/stdin "$pkgdir/usr/share/applications/archtoys.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Archtoys
Comment=System-wide color picker
Exec=archtoys
Icon=archtoys
Terminal=false
Categories=Graphics;Utility;
DESKTOP
  fi

  if [[ -f packaging/archtoys.png ]]; then
    install -Dm644 packaging/archtoys.png "$pkgdir/usr/share/icons/hicolor/256x256/apps/archtoys.png"
    install -Dm644 packaging/archtoys.png "$pkgdir/usr/share/icons/hicolor/512x512/apps/archtoys.png"
    install -Dm644 packaging/archtoys.png "$pkgdir/usr/share/icons/hicolor/1024x1024/apps/archtoys.png"
  elif [[ -f image.png ]]; then
    install -Dm644 image.png "$pkgdir/usr/share/icons/hicolor/256x256/apps/archtoys.png"
  fi
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
