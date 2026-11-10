# Security policy

## Reporting a vulnerability

Please do not open a public issue, pull request or discussion for a security vulnerability, and do not
post exploit details anywhere public.

Use GitHub private vulnerability reporting: open the **Security** tab of this repository and choose
**Report a vulnerability** (https://github.com/slucheninov/mypass-app/security/advisories/new). That
creates a private advisory visible only to you and the maintainers.

Please include:

- which component is affected (the app, the native messaging host, the browser extension, the vault
  engine in `crates/mypass-kdbx`) and the version or commit;
- how you built or installed it;
- steps to reproduce, and what an attacker gains;
- any proof of concept, using a throwaway vault with fake data. Never send real passwords, real vault
  files or key material.

For anything that is not a vulnerability (questions, ideas, ordinary bugs), use the public issue
templates. No other contact address is offered.

## Supported versions

MyPass is pre-release. Only the latest release (until the first release: the latest commit of the
default branch, `main`) is supported; fixes are not backported.

## Scope and known limitations

In scope: the vault engine and the code on top of it (`mypass-kdbx`, `mypass-core`), the Tauri app,
the native messaging host, the browser extension, the bridge between them, the pairing flow and KDBX4
handling, including malformed or hostile `.kdbx` files that crash the parser, exhaust memory or time,
or leak secrets.

Development and unsigned builds are not a supported way to run a real vault. They carry no stable
signing identity, so device-unlock features that depend on it are expected to be limited or
non-functional; reports that only describe that expected limitation are not vulnerabilities. Reports
of a way to read vault contents, keys or tokens from a correctly built and signed release are.

Vulnerabilities in third-party dependencies (listed in `THIRD_PARTY_NOTICES.md`) should be reported
upstream as well; tell us if MyPass is affected in a way the upstream project would not expect.

## Response times and disclosure

This is a small project, so no response-time guarantee is made; we aim to acknowledge a report within
a week. Please give us a reasonable chance to fix a reported issue before you disclose it. Once a fix
is available we publish a GitHub security advisory that credits you, unless you prefer to stay
anonymous.
