# Security policy

## Report a vulnerability

Please don't open a public issue for a security problem. Use GitHub's private vulnerability reporting instead:

1. Open the [Security tab](https://github.com/jordiboehme/rezzer/security) of this repository.
2. Click "Report a vulnerability".
3. Describe the problem and how to reproduce it. A sample file helps a lot.

Only the maintainer can see your report.

## Supported versions

Only the latest release gets security fixes. Please check that the problem still exists there before you report it.

## How Rezzer treats input files

Rezzer reads files from unknown sources, so it treats every input file as untrusted:

- Rezzer never runs code from a file. Macros, scripts and embedded programs are not executed.
- Rezzer limits the size of a file, the depth of nested structures and the time it spends on one file. When a file goes over a limit, Rezzer stops and says so in the output.

A file that gets around one of these limits is a security problem. Please report it as described above.
