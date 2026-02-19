# Goosey

Put `goosey` in your $PATH if you want to launch via:

```
goosey .
```

This will open AGIME GUI from any path you specify

# Unregister Deeplink Protocols (macos only)

`unregister-deeplink-protocols.js` is a script to unregister the deeplink protocol used by AGIME like `goose://`.
This is handy when you want to test deeplinks with the development version of AGIME.

# Usage

To unregister the deeplink protocols, run the following command in your terminal:
Then launch AGIME again and your deeplinks should work from the latest launched application as it is registered on startup.

```bash
node scripts/unregister-deeplink-protocols.js
```

