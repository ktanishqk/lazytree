# lazytree (npm)

Thin Node wrapper around the native LazyTree CLI.

## Install / link

```bash
# From this directory (dev):
npm link

# Or after publishing:
npm i -g lazytree
```

You still need the Rust binary:

```bash
cargo build --release
export PATH="$PWD/../target/release:$PATH"
# or:
export LAZYTREE_BIN=/path/to/lazytree
```

`postinstall` does not download a binary (MVP); it only prints a reminder.

## Usage

```bash
npx lazytree --help
lazytree create my-session
lazytree cursor setup --target ~/src/app
```
