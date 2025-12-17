# raccoin – Nix packaging

This repository provides a Nix-based build setup for **raccoin**,
usable both for **developers** and for **nixpkgs integration**.

* Developers get **fast, reproducible builds** with flakes & direnv
* Non-flake users can still build and hack
* Local `package.nix` for building the release or optionally latest from master
* Upstream (nixpkgs) gets a **minimal, clean `package.nix`** without extra noise
  + can be pushed on nixpkgs after realease

This was written assuming you are using NixOS 25.05 or later.

---

## ⚡ Quickstart

### Build release and install with package.nix

```sh
nix-build -E 'with import <nixpkgs> {}; callPackage ./package.nix {}'
nix profile add ./result
nix profile list # to list the installed package
```

### Build latest and replace installation with package.nix

```sh
nix-build -E 'with import <nixpkgs> {}; callPackage ./package.nix { useLatest = true; }'
nix profile remove raccoin
nix profile add ./result
```

### Build latest from feature branch

```sh
nix-build -E 'with import <nixpkgs> {}; callPackage ./package.nix { useLatest = true; buildBranch = "alby"; }'
```

### With Flakes (recommended)

```sh
# enter dev environment (Rust toolchain, dependencies, etc.)
nix develop

# build raccoin
nix build

# run directly
nix run
```

### Without Flakes

```sh
# enter dev environment
nix-shell

# build raccoin
nix-build

# run binary
./result/bin/raccoin
```

### How to check if Flakes are enabled

If the feature is enabled can be determined with (`nix --version` &ge; 2.20)

`nix config show | grep experimental-features`

To enable it in your configuration.nix

```
  # Enable the Flakes feature and the accompanying new nix command-line tool
  nix.settings.experimental-features = [
    "nix-command"
    "flakes"
  ];
```

---

## Structure

Tree for this repository 

```tree
raccoin-nix/
├── README-nix.md     # documentation
├── package.nix       # local derivation (not for nixpkgs)
├── package-np.nix    # upstream derivation (for nixpkgs only)
├── flake.nix         # flake entrypoint for local development
├── flake.lock        # pin for reproducible builds
├── default.nix       # flake-compat bridge (non-flake builds)
├── shell.nix         # flake-compat bridge (non-flake dev env)
├── flake4default.nix # shim for legacy nix commands
├── .envrc            # direnv integration
└── src/              # optional: Rust sources for raccoin
```

* **`package.nix`** → clean derivation, suitable for upstreaming to nixpkgs.
* **`flake.nix`** → dev-friendly flake entrypoint, imports `package.nix`.
* **`default.nix` & `shell.nix`** → compatibility layers for non-flake users.
* **`.envrc`** → direnv integration (`use flake`).

---

## Usage

### 1. 🚀 With Flakes

Make sure flakes are enabled (`nix --version` &ge; 2.4).

* **Enter dev environment** (with Rust toolchain and dependencies):

  ```bash
  nix develop
  ```

  (or automatically with `direnv` if enabled)

* **Build raccoin**:

  ```bash
  nix build
  ```

* **Run raccoin** directly:

  ```bash
  nix run
  ```

---

### 2. 👴 Without Flakes

If flakes are disabled or unavailable:

* **Enter dev environment**:

  ```bash
  nix-shell
  ```

* **Build raccoin**:

  ```bash
  nix-build
  ```

---

### 3. 𝄞 Integration into nixpkgs

When contributing `raccoin` to nixpkgs, **only `package.nix`** is needed.

Steps:

1. Copy `package.nix` into the appropriate nixpkgs folder (e.g. `pkgs/applications/misc/raccoin/default.nix`).
2. Add `raccoin` to `all-packages.nix`.
3. Run `nix-build -A raccoin` in nixpkgs checkout.

No `flake.nix`, `default.nix`, or other dev files should be included in nixpkgs PRs.

---

## 🛠 Updating

* Update flake sources with:

  ```bash
  nix flake update
  ```
* Or in nixpkgs, use `nix-update`.

---

## 🧑‍💻 Development workflow

* Use `nix develop` + `cargo build` during local hacking.
* The devShell provides `cargo`, `rustc`, and `rust-analyzer`.
* Dependencies are pinned via `flake.lock` for reproducible builds.

---

## 📚 Updating Cargo dependencies

When dependencies change (e.g. after editing `Cargo.toml` or updating the project version), Nix needs a new **cargo vendor hash** (`cargoSha256`).

Steps:

1. **Update Cargo.lock** (if not already):

   ```bash
   cargo update
   ```

2. **Build once with a fake hash**:
   In `package.nix`, set:

   ```nix
   cargoHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
   ```

   (any dummy value works)

   Then run (with flakes enabled):

   ```bash
   nix build
   ```

   or (without flakes enabled)

   ```bash
   nix-build
   ```

3. **Copy the correct hash**:
   The build will fail, printing the expected hash.
   Replace the dummy `cargoHash` with that value.

4. **Rebuild**:

   ```bash
   nix build
   ```

   This time it should succeed.

---

⚡ Tip: inside nixpkgs, you can also run

```bash
nix-update raccoin
```

to automatically bump the version and cargo hash.

If missing, `nix-update` can be run without installaton:
`nix run github:Mic92/nix-update -- raccoin`
or permanently installed: `nix profile install github:Mic92/nix-update`

---
