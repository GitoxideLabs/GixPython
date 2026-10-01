These certificates and the server private key are public, disposable test fixtures.
They authenticate only `localhost` against the included test CA. The CA signing key
was discarded after generation. They expire in September 2036.

The integration tests use Python's standard `ssl` module and loopback sockets;
they do not need OpenSSL commands or external services at test time.
