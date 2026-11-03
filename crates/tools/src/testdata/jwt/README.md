# Test keys

Made for the tests in `jwt.rs` with OpenSSL 3.5, and used nowhere else:
`openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048`, `-algorithm EC
-pkeyopt ec_paramgen_curve:P-256` and `-algorithm ED25519`, with `openssl pkey
-pubout` for the public halves. They are public: never trust a token they sign.
