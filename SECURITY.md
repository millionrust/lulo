# Security policy

Do not open a public issue for a suspected vulnerability, privilege
escalation, lock/authentication bypass, credential or secret exposure,
package/update trust failure, or privacy leak.

Use GitHub's **Security** tab and **Report a vulnerability** to send a private
report to the maintainers. Include the affected revision/package version,
boundary, impact, minimal synthetic reproduction, expected and observed
behavior, and the smallest necessary redacted evidence. Do not include real
credentials, tokens, private keys, document content, personal notifications,
device identifiers, private paths, raw environment dumps, or complete logs.

rmac has no supported public release yet. Maintainers will acknowledge a
complete private report when available, assess severity and affected versions,
prepare a regression test and fix, and coordinate disclosure only after a safe
update or explicit mitigation exists. A report is not considered fixed until
the relevant security-review, package/update, recovery, and supported-hardware
gates pass at the fix revision.
