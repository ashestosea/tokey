# tokey

tokey is a simple keyboard layer mapping tool for linux using evdev.
When the function key is held for a few milliseconds, keys are mapped based on the keymap in the conf file.  
```
[keymap]
KEY_J = "KEY_LEFT"
KEY_L = "KEY_RIGHT"
KEY_I = "KEY_UP"
KEY_K = "KEY_DOWN"
```
tokey also includes dbus messaging by default, allowing you to inspect whether tokey is paused or running.

tokey is inspired by [spacefn](https://github.com/abrasive/spacefn-evdev)

## Installation

Clone this repository and cd into the working copy.  
Then run
```bash
cargo install --path .
```
Then either
- Add `~/.cargo/bin/` to your PATH

or
- `sudo mv ~/.cargo/bin/tokey /usr/bin/`

## Usage

```bash
tokey
```
By default tokey tries to read a configuration file from `~/.config/tokey/conf.toml`  
If not found, tokey writes a default configuration file before running.

```bash
tokey -c "conf_file.toml
```
Use a custom conf file.

```bash
tokey -v
```
Returns version info.

## Configuration

tokey uses [TOML](https://toml.io/en/) v0.5.0 for configuration

```
mode_switch_timeout
```
Time it takes (in ms) to switch into keymapping mode

```
fn_key
```
Key that switches into keymapping mode

```
pause_key
```
Key that toggles tokey on/off (mainly for games)

```
[keymap]
KEY = "MAPPED_KEY"
```
Table containing keymappings.

## License

[WTFPL](http://www.wtfpl.net/about/)
