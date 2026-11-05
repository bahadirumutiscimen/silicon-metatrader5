# Homebrew formula for SiliconMetaTrader5 (desktop app + smt5 CLI), built from source.
#
# Template. The release workflow fills in @VERSION@ / @SHA256@, attaches the
# result to the GitHub release and commits it as Formula/siliconmt5.rb, so this
# repository is its own tap:
#
#   brew tap bahadirumutiscimen/siliconmt5 https://github.com/bahadirumutiscimen/silicon-metatrader5
#   brew trust bahadirumutiscimen/siliconmt5
#   brew install siliconmt5
#
# Built on the user's Mac, so there is no download quarantine and Gatekeeper
# does not block the unsigned app; the bundle gets an ad-hoc signature.
class Siliconmt5 < Formula
  desc "MetaTrader 5 + Python bridge on macOS: desktop app and smt5 CLI"
  homepage "https://github.com/bahadirumutiscimen/silicon-metatrader5"
  url "https://github.com/bahadirumutiscimen/silicon-metatrader5/archive/refs/tags/v@VERSION@.tar.gz"
  sha256 "@SHA256@"
  license "PolyForm-Noncommercial-1.0.0"
  head "https://github.com/bahadirumutiscimen/silicon-metatrader5.git", branch: "main"

  depends_on "rust" => :build
  depends_on :macos

  def install
    cd "app" do
      system "cargo", "build", "--release", "--locked", "-p", "smt5-cli", "-p", "smt5-desktop"
    end
    bin.install "app/target/release/smt5"

    # Runtime files (read-only; settings go to ~/Library/Application Support/SiliconMT5)
    runtime = share/"siliconmt5/runtime"
    runtime.mkpath
    cp_r "runtime/.", runtime
    rm_f runtime/".env"
    rm_r Dir[runtime/"**/__pycache__"]
    cp "packaging/packaged", runtime/".packaged"
    chmod 0755, Dir[runtime/"scripts/*.sh"] + [runtime/"macos/native.sh"]

    # SiliconMetaTrader5.app — the layout `cargo tauri build` would produce
    contents = prefix/"SiliconMetaTrader5.app/Contents"
    (contents/"MacOS").install "app/target/release/siliconmt5"
    (contents/"Resources").install "app/crates/smt5-desktop/icons/icon.icns"
    # a copy, not a symlink: codesign rejects links that leave the bundle
    cp_r runtime, contents/"Resources/runtime"
    (contents/"Info.plist").write <<~XML
      <?xml version="1.0" encoding="UTF-8"?>
      <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
      <plist version="1.0">
      <dict>
        <key>CFBundleName</key><string>SiliconMetaTrader5</string>
        <key>CFBundleDisplayName</key><string>SiliconMetaTrader5</string>
        <key>CFBundleIdentifier</key><string>com.siliconmt5.desktop</string>
        <key>CFBundleExecutable</key><string>siliconmt5</string>
        <key>CFBundleIconFile</key><string>icon</string>
        <key>CFBundlePackageType</key><string>APPL</string>
        <key>CFBundleShortVersionString</key><string>#{version}</string>
        <key>CFBundleVersion</key><string>#{version}</string>
        <key>LSMinimumSystemVersion</key><string>12.0</string>
        <key>LSApplicationCategoryType</key><string>public.app-category.finance</string>
        <key>NSHighResolutionCapable</key><true/>
      </dict>
      </plist>
    XML
    # One ad-hoc identity for the bundle, so macOS remembers permissions
    # (e.g. "control System Events" to bring the MT5 window to the front).
    system "codesign", "--force", "--sign", "-", prefix/"SiliconMetaTrader5.app"
  end

  def caveats
    <<~EOS
      Copy SiliconMetaTrader5 into Applications (Spotlight and Launchpad ignore links):
        smt5 app-install
      After `brew upgrade siliconmt5` the app offers to refresh that copy
      (or run `smt5 app-install` again).

      The app's setup wizard does the rest. From the terminal:
        smt5 doctor

      Settings and the Mac native runtime live in
        ~/Library/Application Support/SiliconMT5
      `brew uninstall siliconmt5` keeps them; remove everything from the app
      (Reset and Remove) before uninstalling.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/smt5 --version")
    assert_path_exists share/"siliconmt5/runtime/macos/native.sh"
    assert_path_exists prefix/"SiliconMetaTrader5.app/Contents/Resources/runtime/compose.yaml"
  end
end
