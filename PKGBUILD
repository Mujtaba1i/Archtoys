pkgname=archtoys
pkgver=0.1.1
pkgrel=1
pkgdesc="PowerToys-like color picker for Linux (Slint-based)"
arch=('x86_64')
url="https://github.com/Mujtaba1i/Archtoys"
license=('MIT')
depends=('glibc')
makedepends=('cargo' 'git')
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

  install -Dm644 packaging/archtoys.desktop "$pkgdir/usr/share/applications/archtoys.desktop"
  install -Dm644 packaging/archtoys.png "$pkgdir/usr/share/icons/hicolor/256x256/apps/archtoys.png"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
