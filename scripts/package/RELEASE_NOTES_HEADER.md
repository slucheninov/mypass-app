MyPass {{VERSION}} for Apple Silicon (macOS 14+).

Signed with the team's Apple Development certificate; not notarized.

Install:

1. Remove the quarantine flag from the downloaded DMG:
   `xattr -dr com.apple.quarantine ~/Downloads/MyPass-{{VERSION}}-macos-arm64.dmg`
2. Open the DMG and drag MyPass.app to Applications.
3. Remove the quarantine flag from the installed app:
   `sudo xattr -dr com.apple.quarantine /Applications/MyPass.app`
4. Quit the old Swift MyPass app, then start MyPass from /Applications.

Browser extension:

1. Extract `MyPass-chrome-extension-{{VERSION}}.zip`.
2. Open `chrome://extensions` and turn on Developer mode.
3. Click Load unpacked and choose the extracted folder.
4. Pair in MyPass Settings > Browser.

Verify the downloads with `shasum -a 256 -c SHA256SUMS.txt`.
