# Binary Documentation

The `imessage-exporter` binary exports iMessage data to `txt`, `html`, or `pdf` formats. It can also run diagnostics to find problems with the iMessage database.

## Installation

There are several ways to install this software.

### Cargo (recommended)

This binary is available on [crates.io](https://crates.io/crates/imessage-exporter).

`cargo install imessage-exporter` is the best way to install the app for normal use.

<details><summary>Upgrade steps</summary><p><pre>$ cargo uninstall imessage-exporter
$ cargo install imessage-exporter</pre></p></details>

<details><summary>Uninstall steps</summary><p><pre>$ cargo uninstall imessage-exporter</pre></p><p>Optional: uninstall Rust<pre>$ rustup self uninstall</pre></p></details>

### Homebrew

This binary is available via [`brew`](https://formulae.brew.sh/formula/imessage-exporter).

`brew install imessage-exporter` will install the app, but it may not be up to date with the latest release.

<details><summary>Upgrade steps</summary><p><pre>$ brew upgrade</pre></p></details>

<details><summary>Uninstall steps</summary><p><pre>$ brew uninstall imessage-exporter</pre></p></details>

### Prebuilt Binaries

The [releases page](https://github.com/ReagentX/imessage-exporter/releases) provides prebuilt binaries for both Apple Silicon and Intel-based Macs.

<details><summary>Upgrade steps</summary><p>Download new releases as available</p></details>

<details><summary>Uninstall steps</summary><p><pre>$ rm path/to/imessage-exporter-binary</pre></p></details>

### Installing manually

- `clone` the repository
- `cd` to the repository
- `cargo run --release` to compile

## How To Use

```txt
-d, --diagnostics
        Print diagnostic information and exit
        
-f, --format <txt, html, pdf>
        Specify a single file format to export messages into
        
-c, --copy-method <clone, basic, full, disabled>
        Specify an optional method to use when copying message attachments
        `clone` will copy all files without converting anything
        `basic` will copy all files and convert HEIC images to JPEG
        `full` will copy all files and convert HEIC files to JPEG, CAF to MP4, and MOV to MP4
        If omitted, the default is `disabled`
        ImageMagick is required to convert images on non-macOS platforms
        ffmpeg is required to convert audio on non-macOS platforms and video on all platforms
        
-p, --db-path <path/to/source>
        Specify an optional custom path for the iMessage database location
        For macOS, specify a path to a `chat.db` file
        For iOS, specify a path to the root of a device backup directory
        If the iOS backup is encrypted, --cleartext-password can be passed or you will be prompted for the password
        If omitted, the default directory is ~/Library/Messages/chat.db
        
-r, --attachment-root <path/to/messages/root>
        Specify an optional custom path to look for attachment data in
        Only use this if attachments are stored separately from the database's default location
        The provided path should be absolute
        This option affects both the `Attachments` and `StickerCache` directories
        Also works with jailbroken iOS sms.db databases (use `--platform macOS`)
        Has no effect on iOS backups
        The default location is ~/Library/Messages
        
-a, --platform <macOS, iOS>
        Specify the platform the database was created on
        If omitted, the platform type is determined automatically
        
-o, --export-path <path/to/save/files>
        Specify an optional custom directory for outputting exported data
        If omitted, the default directory is ~/imessage_export
        
-s, --start-date <YYYY-MM-DD>
        The start date filter
        Only messages sent on or after this date will be included
        
-e, --end-date <YYYY-MM-DD>
        The end date filter
        Only messages sent before this date will be included
        
-l, --no-lazy
        Do not include `loading="lazy"` in HTML export `img` tags
        This will make pages load slower but PDF generation work
        
-m, --custom-name <custom-name>
        Specify an optional custom name for the database owner's messages in exports
        Conflicts with --use-caller-id
        
-i, --use-caller-id
        Use the database owner's caller ID in exports instead of "Me"
        Conflicts with --custom-name
        
-b, --ignore-disk-warning
        Bypass the disk space check when exporting data
        By default, exports will not run if there is not enough free disk space
        
-t, --conversation-filter <filter>
        Filter exported conversations by contact names, numbers, or emails
        To provide multiple filter criteria, use a comma-separated string
        All conversations with the specified participants are exported, including group conversations
        Example: `-t steve@apple.com,5558675309`
        
-x, --cleartext-password <password>
        Optional password for encrypted iOS backups
        This is only used when the source is an encrypted iOS backup directory
        If omitted on an encrypted backup, you will be prompted for the password (recommended)
        A password provided with this option is visible on screen, in the process table, and in your shell history
        
-n, --contacts-path <path>
        Optional custom path for a macOS or iOS contacts database file
        This should be resolved automatically, but can be manually provided
        Handles from the messages table will be mapped to names in the provided database
        Generally, one of `AddressBook-v22.abcddb` or `AddressBook.sqlitedb`
        
    --no-progress
        Disable the on-screen progress bar regardless of context
        By default, the progress bar is shown only when stderr is a terminal,
        so headless invocations (CI, output redirected to a logfile) stay clean automatically.
        Use this flag to suppress the bar even in an interactive terminal.
        
    --max-image-size <pixels>
        PDF export only: cap the longest edge of image attachments to this many pixels before embedding
        Larger images are downscaled with `sips` (macOS) or ImageMagick; smaller images are left untouched
        This is the dominant lever on PDF size
        [default: 1600]
        
    --image-quality <quality>
        PDF export only: JPEG quality (1-100) for images embedded in the PDF
        The browser embeds images losslessly; they are re-encoded to JPEG at this quality to keep the PDF small
        [default: 80]
        
    --chrome-path <path/to/chrome>
        PDF export only: explicit path to a Chrome, Chromium, or Edge binary
        If omitted, common install locations are probed automatically
        
    --keep-html
        PDF export only: keep the intermediate per-conversation HTML files next to the generated PDFs
        By default they are removed after a successful conversion
        
    --pdf-engine <quartz, chrome>
        PDF export only: rendering engine to use
        `quartz` (macOS only, default there) renders with WebKit + Apple's Quartz PDF engine: much smaller, searchable PDFs with embedded JPEGs preserved
        `chrome` renders with a headless Chrome/Chromium/Edge browser and works on every platform
        
-h, --help
        Print help
-V, --version
        Print version
```

### Examples

Export as `html` and copy attachments in web-compatible formats from the default iMessage Database location to your home directory:

```zsh
imessage-exporter -f html -c full
```

Export as `pdf` (one PDF per conversation) and copy attachments from the default iMessage Database location to a new folder called `pdf_export`. On macOS this uses the built-in Quartz engine; on other platforms pass `--pdf-engine chrome` (a headless Chrome/Chromium/Edge install is required):

```zsh
imessage-exporter -f pdf -c full -o pdf_export
```

Export as `txt` and copy attachments in their original formats from the default iMessage Database location to a new folder in the current working directory called `output`:

```zsh
imessage-exporter -f txt -o output -c clone
```

Export as `txt` from an iPhone backup located at `~/iphone_backup_latest` to a new folder in the current working directory called `backup_export`:

```zsh
imessage-exporter -f txt -p ~/iphone_backup_latest -a iOS -o backup_export
```

Export as `html` from `/Volumes/external/chat.db` to `/Volumes/external/export` without copying attachments:

```zsh
imessage-exporter -f html -c disabled -p /Volumes/external/chat.db -o /Volumes/external/export
```

Export as `html` from `/Volumes/external/chat.db` to `/Volumes/external/export` with attachments in `/Volumes/external/Attachments`:

```zsh
imessage-exporter -f html -c clone -p /Volumes/external/chat.db -r /Volumes/external/Attachments -o /Volumes/external/export 
```

Export messages from `2020-01-01` to `2020-12-31` as `txt` from the default macOS iMessage Database location to `~/export-2020`:

```zsh
imessage-exporter -f txt -o ~/export-2020 -s 2020-01-01 -e 2021-01-01 -a macOS
```

Export messages from a specific participant as `html` and copy attachments in their original formats from the default iMessage Database location to your home directory:

```zsh
imessage-exporter -f html -c clone -t "5558675309"
```

Export messages from a specific participant's name as `txt` and without attachments from the default iMessage Database location to your home directory:

```zsh
imessage-exporter -f txt -t "Steve Jobs"
```

Export messages from multiple specific participants as `html` without attachments from the default iMessage Database location to your home directory:

```zsh
imessage-exporter -f html -t "5558675309,steve@apple.com"
```

Export messages from participants matching a specific country and area code as `html` without attachments from the default iMessage Database location to your home directory:

```zsh
imessage-exporter -f html -t "+1555"
```

Export messages from participants using email addresses but not phone numbers as `html` without attachments from the default iMessage Database location to your home directory:

```zsh
imessage-exporter -f html -t "@"
```

## Features

[Click here](../docs/features.md) for a full list of features.

## Caveats

### Cross-platform attachment conversion

[ImageMagick](https://imagemagick.org/index.php) is required to make exported images more compatible on non-macOS platforms.

[ffmpeg](https://ffmpeg.org) is required to make exported audio more compatible on non-macOS platforms and exported video more compatible on all platforms.

### Contacts

`imessage-exporter` will automatically attempt to resolve handle details (email addresses and phone numbers) against contacts found either in the provided iOS backup or on the local macOS Address Book. Users can optionally pass in a path to an Address Book database, but this should generally not be necessary.

### HTML Exports

In HTML exports in Safari, when referencing files in-place, you must permit Safari to read from the local file system in the `Develop > Developer Settings...` menu:

![](../docs/binary/img/safari_local_file_restrictions.png)

Further, since the files are stored in `~/Library`, you will need to grant your browser Full Disk Access in System Settings.

Note: This is not required when passing a valid `--copy-method`.

#### Custom Styling for HTML Exports

You can customize the appearance of HTML exports by creating your own CSS file:

1. Create a file named `style.css` in the same directory as your exported files
2. Add your custom styles to this file
3. These styles will be automatically applied to your exported HTML files

Since custom styles are loaded after the default styles, they should automatically override rules with the same specificity.

##### Example Custom CSS

For example, to prevent messages from breaking across pages when printing:

```css
.message {
    break-inside: avoid;
}
```

The default styles can be viewed [here](src/exporters/html/resources/style.css).

### PDF Exports

Run `-f pdf` to export each conversation as its own PDF. The exporter renders the same HTML it would otherwise write, downscales image attachments so the embedded images stay small, and prints each conversation to PDF — splitting very large conversations into chunks that it merges back into a single document.

Two rendering engines are available via `--pdf-engine`:

- **`quartz`** (macOS only, and the default there) renders with WebKit and Apple's Quartz PDF engine. It produces much smaller, searchable PDFs and preserves the embedded JPEGs as-is.
- **`chrome`** renders with a headless Chrome, Chromium, or Edge browser and works on every platform. Install one of those browsers, or point at a binary with `--chrome-path`.

Tune output size with `--max-image-size` (longest image edge, default `1600`px) and `--image-quality` (JPEG quality, default `80`). Pass `--keep-html` to keep the intermediate per-conversation HTML next to the PDFs instead of deleting it.

The intermediate HTML is written into your export directory, so PDF export refuses to run when that directory already contains `.html` files (to avoid appending to or skipping them). Export to an empty directory, or move existing `.html` files first.

#### History

Earlier attempts shelled out to `wkhtmltopdf` (which refused to render local images even with `--enable-local-file-access`) and to the `headless-chrome` crate (whose sync implementation [timed out](https://github.com/atroche/rust-headless-chrome/issues/319) on large PDFs, while its async wrappers bloated the binary). The current approach shells out to a browser or the Quartz helper directly, so it pulls in no heavy Rust PDF dependency.
