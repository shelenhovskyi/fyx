# Test TLS material (tests only)

A throwaway CA and a `localhost` / `127.0.0.1` server certificate for the `network:fetch` engine
tests (`network.rs`, `test_server::start_tls`). The key is public on purpose: nothing outside the
tests trusts this CA. P-256, valid until 2126. Regenerate with openssl:

    openssl ecparam -name prime256v1 -genkey -noout -out ca.key
    openssl req -new -key ca.key -subj "/CN=Fyx test CA (ui_ext_wasm tests only)" -out ca.csr
    openssl x509 -req -in ca.csr -signkey ca.key -days 36500 -extfile ca.ext -out ca.pem
    openssl ecparam -name prime256v1 -genkey -noout -out leaf.key
    openssl req -new -key leaf.key -subj "/CN=localhost" -out leaf.csr
    openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 36500 \
        -extfile leaf.ext -out leaf.pem
    openssl x509 -in ca.pem -outform DER -out ca.der
    openssl x509 -in leaf.pem -outform DER -out localhost.der
    openssl pkcs8 -topk8 -nocrypt -in leaf.key -outform DER -out localhost.key.der

`ca.ext`: `basicConstraints = critical, CA:TRUE`, `keyUsage = critical, keyCertSign, cRLSign`,
`subjectKeyIdentifier = hash`. `leaf.ext`: `basicConstraints = critical, CA:FALSE`,
`keyUsage = critical, digitalSignature`, `extendedKeyUsage = serverAuth`,
`subjectAltName = DNS:localhost, IP:127.0.0.1`.
