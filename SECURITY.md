# Security policy

Please report a vulnerability privately, through
[Report a vulnerability](https://github.com/husur/android-resources-lsp/security/advisories/new),
rather than in a public issue. The language server only reads files of your project and sends
nothing over the network, but the extension downloads and runs its binary: anything that could make
it run another one is in scope.

Only the latest release is supported. The extension downloads the newest language server when Zed
starts, so a fix reaches every user without an update of the extension (an `android-resources-lsp`
found in your `PATH` takes precedence, and has to be updated by hand).
