# Wallpaper Switcher

A see-through wallpaper picker for Windows 10 and 11. Press a shortcut, flick through your wallpapers, hit Enter.

## Install

1. Download `Wallpaper Switcher Setup 1.1.0.exe` and run it.
2. Windows may show "Windows protected your PC" because the app isn't code-signed yet. Click **More info**, then **Run anyway**.
3. Click **Install**. No admin rights needed: it installs just for your account.

On first launch you'll see a short welcome screen with your shortcut and wallpaper folder. After that it waits in the tray near the clock.

To remove it, open **Settings → Apps → Installed apps**, find Wallpaper Switcher and choose **Uninstall**. Your wallpapers aren't touched.

## Use it

| Key | Does |
| --- | --- |
| `Ctrl` `Alt` `W` | Open or close (you can change this in settings) |
| `←` `→`, mouse wheel | Browse the row: folders first, then loose wallpapers |
| `↓` or `Enter` on a folder | Open it |
| `↑` `↓` inside a folder | Scroll through the folder |
| `Esc`, `←` `→`, or `↑` at the top | Close the folder |
| `Enter`, or click the selected one | Set wallpaper (on every screen if you have several) |
| `Shift` `Enter` | Set it on the screen you're editing only |
| `Ctrl` + arrows, `Ctrl` `1`–`9` | Edit another screen |
| `Ctrl` `L` | Edit the lock screen, when it has its own wallpaper |
| `Tab` | Switch between row and grid |
| Any letter | Filter by name |
| `Ctrl` `H` | Hide the interface (you choose which parts in settings) |
| `Ctrl` `,` | Settings, with `Ctrl` `Tab` to switch tabs |
| `Esc`, or click empty space | Clear the filter, then close |

Wallpapers live in `Pictures\Wallpapers` (created on first run), or any folder you pick in settings. Each folder inside it shows up as a stack. JPG, PNG, WebP and BMP work.

Settings is split into three tabs:

- **General**: wallpaper folder, shortcut, Start with Windows, and full-quality JPEGs (stops Windows from recompressing JPEG wallpapers to about 85%)
- **Appearance**: row or grid, how much the background is darkened, and what `Ctrl` `H` hides
- **Screens**: everything about multiple monitors and the lock screen

## Multiple screens

With more than one monitor, a map of your screens sits in the bottom-right corner, drawn to scale with each screen's current wallpaper on it. The switcher opens on the screen your mouse is on.

- `Enter` sets the wallpaper on every screen and `Shift` `Enter` only on the one you're editing. A setting swaps the two.
- Click a screen on the map, or use `Ctrl` + arrows or `Ctrl` `1`–`9`, to edit another screen. The switcher moves there and your mouse follows. Turn off **Move to the screen you edit** to keep it where you opened it.
- The screen you switch to flashes briefly so you can see which one it is. This can be turned off.
- The map comes in three sizes.

## Lock screen

**Settings → Screens → Lock screen** can leave the lock screen alone (the default), keep it in step with your main screen, or give it its own wallpaper. With its own wallpaper, a lock screen tile joins the map, bottom left or above it. Click it or press `Ctrl` `L`, then pick a wallpaper and press `Enter`.

If the lock screen doesn't change, check that it's set to **Picture** rather than Windows spotlight under **Settings → Personalization → Lock screen** in Windows.

The switcher shows over windowed and borderless games. Games running in exclusive fullscreen don't allow anything on top of them.

## Where it keeps things

Everything is per user, so nothing depends on a particular PC or username:

- App: `%LOCALAPPDATA%\Programs\Wallpaper Switcher`
- Settings: `%APPDATA%\com.wallpaperswitcher.app\settings.json`
- Thumbnail cache: `%LOCALAPPDATA%\com.wallpaperswitcher.app`

Uninstalling removes all three.

## Build it yourself

On Windows, install [Rust](https://rustup.rs), [Node.js](https://nodejs.org) and [NSIS](https://nsis.sourceforge.io), then:

```sh
npm install
npm run tauri build
makensis installer\wallpaper-switcher.nsi
```

The installer is written to the `installer` folder.

- `src` is the overlay itself (HTML, CSS and JS, with the two fonts it uses)
- `src-tauri/src` is the Rust side: window, tray, shortcut, thumbnails, setting the wallpaper and lock screen, the screen flash and start-up entry
- `installer` has the NSIS script and its artwork

The fonts, Bodoni Moda and Schibsted Grotesk, are under the SIL Open Font License. Their license files are in `src/fonts`.
