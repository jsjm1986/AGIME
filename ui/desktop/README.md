# AGIME Desktop App

Native desktop app for AGIME built with [Electron](https://www.electronjs.org/) and [ReactJS](https://react.dev/).

# Building and running

```
git clone git@github.com:jsjm1986/AGIME.git
cd AGIME
source ./bin/activate-hermit
cd ui/desktop
npm install
npm run start
```

## Platform-specific build requirements

### Linux
For building on Linux distributions, you'll need additional system dependencies:

**Debian/Ubuntu:**
```bash
sudo apt install dpkg fakeroot
```

**Arch/Manjaro:**
```bash
sudo pacman -S dpkg fakeroot
```

**Fedora/RHEL:**
```bash
sudo dnf install dpkg-dev fakeroot
```

# Building notes

This is an electron forge app, using vite and react.js. `agimed` runs as multi process binaries on each window/tab similar to chrome.

## Building for different platforms

### macOS
`npm run bundle:default` will give you a agime.app/zip which is signed/notarized but only if you setup the env vars as per `forge.config.ts` (you can empty out the section on osxSign if you don't want to sign it) - this will have all defaults.

`npm run bundle:preconfigured` will make a agime.app/zip signed and notarized, but use the following:

```python
            f"        process.env.AGIME_PROVIDER__TYPE = '{os.getenv("AGIME_BUNDLE_TYPE")}';",
            f"        process.env.AGIME_PROVIDER__HOST = '{os.getenv("AGIME_BUNDLE_HOST")}';",
            f"        process.env.AGIME_PROVIDER__MODEL = '{os.getenv("AGIME_BUNDLE_MODEL")}';"
```

This allows you to set for example AGIME_PROVIDER__TYPE to be "databricks" by default if you want (so when people start the app - they will get that out of the box). There is no way to set an api key in that bundling as that would be a terrible idea, so only use providers that can do oauth (like databricks can), otherwise stick to default AGIME.

### Linux
For Linux builds, first ensure you have the required system dependencies installed (see above), then:

1. Build the Rust backend:
```bash
cd ../..  # Go to project root
cargo build --release -p agime-server
```

2. Copy the server binary to the expected location:
```bash
mkdir -p src/bin
cp ../../target/release/agimed src/bin/
```

3. Build the application:
```bash
# For ZIP distribution (works on all Linux distributions)
npm run make -- --targets=@electron-forge/maker-zip

# For DEB package (Debian/Ubuntu)
npm run make -- --targets=@electron-forge/maker-deb
```

The built application will be available in:
- ZIP: `out/make/zip/linux/x64/agime-linux-x64-{version}.zip`
- DEB: `out/make/deb/x64/agime_{version}_amd64.deb`
- Executable: `out/agime-linux-x64/agime`

### Windows
Use the existing Windows build process as documented.


# Running with agimed server from source

Set `VITE_START_EMBEDDED_SERVER=yes` to no in `.env`.
Run `cargo run -p agime-server` from parent dir.
`npm run start` will then run against this.
You can try server directly with `./test.sh`
