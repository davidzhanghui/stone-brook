#!/bin/bash
# 石渠一键构建：环境检查 → 格式 / lint 检查 → 单元测试 → release 编译 → 组装 .app（ad-hoc 签名）→ 制作 DMG。
#
# 用法：scripts/build.sh [选项]
#   --fast        跳过格式、clippy 与单元测试，只编译打包
#   --dev         正式包里也带上开发功能（脚本测试 STONEBROOK_SCRIPT、压测菜单）
#   --no-dmg      只生成 dist/石渠.app，不做 DMG
#   --run         打包完成后启动（已在运行的石渠会先退出）
#   -h, --help    显示本帮助
#
# 产物：dist/石渠.app、dist/石渠-<版本>.dmg
set -euo pipefail
cd "$(dirname "$0")/.."

FAST=0 DEV=0 DMG=1 RUN=0
for arg in "$@"; do
  case "$arg" in
    --fast) FAST=1 ;;
    --dev) DEV=1 ;;
    --no-dmg) DMG=0 ;;
    --run) RUN=1 ;;
    -h | --help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "未知选项：$arg（--help 查看用法）" >&2; exit 2 ;;
  esac
done

step() { printf '\n\033[1;34m▸ %s\033[0m\n' "$*"; }
fail() { printf '\033[1;31m✘ %s\033[0m\n' "$*" >&2; exit 1; }
START=$(date +%s)

# ───────────── 环境检查 ─────────────
step "检查构建环境"
[ "$(uname -s)" = "Darwin" ] || fail "石渠只支持 macOS（依赖 AppKit / WKWebView / AVFoundation）"
command -v cargo >/dev/null || fail "没有找到 cargo，请先安装 Rust：https://rustup.rs"
xcode-select -p >/dev/null 2>&1 || fail "没有 Xcode 命令行工具，请先执行：xcode-select --install"
RUST_VER=$(rustc --version | awk '{print $2}')
printf '  macOS %s · rustc %s · %s\n' "$(sw_vers -productVersion)" "$RUST_VER" "$(uname -m)"
case "$RUST_VER" in 1.[0-7]?.* | 1.8[0-4].*) fail "需要 Rust 1.85 以上（当前 $RUST_VER），请执行 rustup update" ;; esac

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
APP="dist/石渠.app"
FEATURES=(--no-default-features)
[ "$DEV" = 1 ] && FEATURES=()

# ───────────── 检查与测试 ─────────────
if [ "$FAST" = 0 ]; then
  step "检查代码格式（cargo fmt）"
  cargo fmt --check >/dev/null || fail "代码格式不一致，请先执行 cargo fmt"

  step "静态检查（cargo clippy，零警告）"
  LOG=$(mktemp)
  cargo clippy --all-targets --quiet -- -D warnings >"$LOG" 2>&1 || { grep -E -A8 '^error' "$LOG" >&2; rm -f "$LOG"; fail "clippy 有警告或错误"; }
  rm -f "$LOG"

  step "单元测试（cargo test）"
  if ! cargo test --quiet 2>&1 | tail -n 4; then fail "单元测试未通过"; fi
fi

# ───────────── 编译 ─────────────
step "编译 release 版本 ${VERSION}${FEATURES[*]:+（不含开发功能）}"
cargo build --release --locked "${FEATURES[@]}"

# ───────────── 组装 .app ─────────────
step "组装 ${APP}"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/stonebrook "$APP/Contents/MacOS/Stonebrook"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
# 画板素材的第三方许可全文（MIT / Apache-2.0 要求随分发附带）
cp assets/iconlib/THIRD_PARTY_LICENSES.txt "$APP/Contents/Resources/THIRD_PARTY_LICENSES.txt"
cp assets/fonts/LICENSE-StonebrookSketch.txt "$APP/Contents/Resources/LICENSE-StonebrookSketch.txt"
cat >"$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>石渠</string>
  <key>CFBundleDisplayName</key><string>石渠</string>
  <key>CFBundleIdentifier</key><string>app.stonebrook</string>
  <key>CFBundleExecutable</key><string>Stonebrook</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleDevelopmentRegion</key><string>zh_CN</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>NSHumanReadableCopyright</key><string>石渠 Stonebrook · 藏书于石，引水成渠</string>
</dict>
</plist>
PLIST

step "ad-hoc 签名"
codesign --force --deep --sign - "$APP" >/dev/null 2>&1
codesign --verify "$APP" || fail "签名校验失败"

# ───────────── DMG ─────────────
if [ "$DMG" = 1 ]; then
  step "制作 DMG"
  DMG_PATH="dist/石渠-${VERSION}.dmg"
  STAGE=$(mktemp -d)
  trap 'rm -rf "$STAGE"' EXIT
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  rm -f "$DMG_PATH"
  hdiutil create -volname "石渠" -srcfolder "$STAGE" -ov -format UDZO "$DMG_PATH" >/dev/null
fi

step "完成（用时 $(($(date +%s) - START)) 秒）"
du -sh "$APP" 2>/dev/null | sed 's/^/  /'
[ "$DMG" = 1 ] && du -sh "dist/石渠-${VERSION}.dmg" 2>/dev/null | sed 's/^/  /'

# ───────────── 启动 ─────────────
if [ "$RUN" = 1 ]; then
  if pgrep -x Stonebrook >/dev/null; then
    echo "  石渠正在运行，先退出旧进程"
    osascript -e 'quit app id "app.stonebrook"' >/dev/null 2>&1 || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do pgrep -x Stonebrook >/dev/null || break; sleep 0.5; done
    pkill -x Stonebrook 2>/dev/null || true
  fi
  open "$APP"
  echo "  已启动 $APP"
else
  echo "  拖动 $APP 到「应用程序」即可使用；加 --run 可直接启动"
fi
