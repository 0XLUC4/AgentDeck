# Bundled ConPTY

`conpty.dll` and `OpenConsole.exe` (x64), unmodified, from Microsoft's NuGet package
[Microsoft.Windows.Console.ConPTY](https://www.nuget.org/packages/Microsoft.Windows.Console.ConPTY)
1.25.260930003 (Authenticode-signed by Microsoft Corporation). Source: https://github.com/microsoft/terminal,
MIT license.

Embedded in the exe and extracted on first use by `crates/core/src/pty/conpty.rs`.
To update: download the new `.nupkg`, copy `runtimes/win-x64/native/conpty.dll` and
`build/native/runtimes/x64/OpenConsole.exe` here, and bump `VERSION` in `conpty.rs`.
