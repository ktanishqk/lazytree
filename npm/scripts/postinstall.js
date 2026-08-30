'use strict';
// Soft postinstall: remind how to get the native binary. No network download in MVP.
console.log(
  '[lazytree] npm wrapper installed. Build the Rust CLI (`cargo build --release`) and put it on PATH, or set LAZYTREE_BIN.',
);
