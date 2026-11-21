# Security

Lumio reads and writes files on your drives and can hold API keys (in your system keychain), so security problems matter.

## Reporting a vulnerability

Please **don't** open a public issue. Use GitHub's private reporting instead: go to the repository's **Security** tab and choose **Report a vulnerability**. Include what you found, how to reproduce it and which version and system you used.

You should hear back within two weeks. This is a spare-time project, so a fix may take longer, but we'll keep you updated and credit you in the release notes if you'd like.

## Supported versions

Only the latest release gets security fixes.

## Scope

Things we especially want to hear about: anything that lets a web page, photo file or metadata run code or reach files outside the library; the local video server on Linux (`videoserver.rs`) answering anyone but Lumio; API keys or SmugMug tokens leaking; and file operations that could lose or overwrite photos.
