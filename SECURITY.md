# Security

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's private vulnerability reporting: open the **Security** tab of this repository and click **Report a vulnerability**. Do not open a public issue for a vulnerability.

There is no e-mail address for reports.

## What to expect

Hush is a one-person hobby project. There is no security team and no promised response time. Reports are read and fixed when time allows.

## Scope

The vault protects the file at rest: it is a SQLCipher database, and the key is derived from your passphrase with PBKDF2-HMAC-SHA512. It does not protect:

- a running app that is unlocked,
- a machine that runs malware, or
- a weak passphrase.

Reports about those limits are not vulnerabilities, but a report showing that the vault file can be read without the passphrase, or that the passphrase or key is written to disk, is very welcome.
