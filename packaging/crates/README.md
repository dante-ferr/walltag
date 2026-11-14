# crates.io

`Cargo.toml` already has the metadata crates.io needs (`license`, `repository`,
`description`, `readme`, keywords, categories).

## Publish

1. Create an account at https://crates.io and verify email.
2. Create an API token at https://crates.io/settings/tokens
3. Log in once:

   ```bash
   cargo login
   ```

4. Dry-run, then publish:

   ```bash
   cargo publish --dry-run
   cargo publish
   ```

Users install with:

```bash
cargo install walltag
```

They still need system GTK 4 / Libadwaita development libraries to compile.
