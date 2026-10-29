# Android Resources for Zed

Shows the text of Android string resources in Kotlin sources, the way Android Studio folds
`stringResource(R.string.title)` into `"Alerte"`.

- **Inlay hint**: the text appears right after each `R.string.*` and `R.plurals.*` reference (the
  `other` quantity for plurals), cut after 40 characters.
- **Hover**: the full text, and every quantity of plurals.
- **Go to definition**: jumps to the string in its `strings.xml`.
- **Warning**: a name that no `res/values` file defines is underlined.

## Installation

Run `zed: extensions`, search for **Android Resources** and install it. The language server is
downloaded automatically.

Then make sure that:

- the **Kotlin** extension is installed, since Zed only starts the server on files it knows as
  Kotlin;
- inlay hints are enabled, as Zed turns them off by default. Run `editor: toggle inlay hints`, or add
  this to your settings:

  ```json
  "inlay_hints": { "enabled": true }
  ```

## Where the texts come from

The server reads every XML file in a `res/values` directory of your project, in any module or
source set, skipping `build` and hidden directories. Saving one of them updates the hints and the
warnings.

A name defined more than once, say in `src/main` and overridden in `src/debug`, or in two modules,
is not resolved the way Gradle merges resources: the hover lists every definition and go to
definition offers all of them, the closest to the Kotlin file first. The hint shows that closest
one, so a source in `src/debug` gets the `debug` text.

## Limits

- Only the default `res/values` is read: translations (`values-fr`, …) are not shown.
- Compose Multiplatform resources (`Res.string.*`, from `composeResources`) are not supported.
- `<string-array>` and the other resource types are not read.
- The warning skips `R` written with its package (`com.example.lib.R.string.title`), which may
  come from a library, and commented-out code. It can still flag a string that only exists in a
  library or in a `resValue` of a Gradle script, which is why it is a warning and not an error.

## Contributing

Bug reports and pull requests are welcome: [CONTRIBUTING.md](CONTRIBUTING.md) explains how to build,
test and release the extension.

## License

Licensed under the [Apache License 2.0](LICENSE).

The language server binaries also contain third-party software: its licenses are in
`THIRD-PARTY-LICENSES.md` and `COPYRIGHT-library.html`, attached to each release.
