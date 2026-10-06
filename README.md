# Wallpaper Switcher

A see-through wallpaper picker for Windows 10 and 11. Press a shortcut, flick through your wallpapers, hit Enter.

## Install

1. Download `Wallpaper Switcher Setup 1.0.0.exe` and run it.
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
| `Enter`, or click the selected one | Set wallpaper |
| `Tab` | Switch between row and grid |
| Any letter | Filter by name |
| `Ctrl` `,` | Settings |
| `Esc`, or click empty space | Clear the filter, then close |

Wallpapers live in `Pictures\Wallpapers` (created on first run), or any folder you pick in settings. Each folder inside it shows up as a stack. JPG, PNG, WebP and BMP work.

Settings also has row or grid as the default view, how much the background is darkened, Start with Windows, and full-quality JPEGs (stops Windows from recompressing JPEG wallpapers to about 85%).

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
- `src-tauri/src` is the Rust side: window, tray, shortcut, thumbnails, setting the wallpaper and start-up entry
- `installer` has the NSIS script and its artwork

The fonts, Bodoni Moda and Schibsted Grotesk, are under the SIL Open Font License. Their license files are in `src/fonts`.
