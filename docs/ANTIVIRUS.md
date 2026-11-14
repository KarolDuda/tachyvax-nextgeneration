# Antivirus false positives

NEOX is unsigned, new and touches the registry and power settings, which is exactly what some antivirus heuristics dislike. We do **not** hide, pack or obfuscate anything. What we do instead:

- The installer and `Neox.exe` carry full version information (publisher, product, copyright).
- Builds come only from GitHub Actions. Each release has SHA-256 hashes in its notes and a build provenance attestation: `gh attestation verify Neox_Setup.exe --repo KarolDuda/NEOX`.
- The program never needs administrator rights to start (`asInvoker`); it asks only when a change needs them.
- No packer, no self-modifying code, no hidden processes beyond the PowerShell/CIM queries it documents.

## If a scanner flags a release

1. Upload `Neox_Setup.exe` to virustotal.com and note which engines flag it (usually 1-3 generic ML names).
2. Report it as a false positive: Microsoft https://www.microsoft.com/wdsi/filesubmission (choose "Software developer"), and each flagging vendor's false-positive form.
3. A code-signing certificate is the real fix (it also removes SmartScreen and Smart App Control blocks). Options: Azure Trusted Signing, an OV certificate, or SignPath Foundation for open-source projects.
