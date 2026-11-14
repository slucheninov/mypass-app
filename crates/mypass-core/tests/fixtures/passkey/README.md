# Passkey test fixtures

Synthetic data only. No key here protects anything real.

| File | Contents |
|---|---|
| `chrome_create_request.json`, `chrome_get_request.json` | `requestDetailsJson` shapes as Chrome's `webAuthenticationProxy` hands them over |
| `rfc6979_a25_sec1.pem` | The RFC 6979 A.2.5 P-256 private key (`x = C9AFA9D8...120F6721`) as SEC1 (`BEGIN EC PRIVATE KEY`) |
| `rfc6979_a25_pkcs8.pem` | The same key as PKCS#8 (`BEGIN PRIVATE KEY`), the form MyPass and KeePassXC store |
| `p384_pkcs8.pem` | A throwaway P-384 key, only for the "wrong curve is rejected" tests |
| `keepassxc_style_entry.json` | Entry fields written the way KeePassXC writes a passkey: PKCS#8 PEM, no `KPEX_PASSKEY_CREATED`, no `KPEX_PASSKEY_FLAG_BE/BS`, a 16-byte credential id; one copy with unpadded and one with padded base64url |

The PEM files were made once with OpenSSL and committed as text:

```sh
# SEC1 DER of the A.2.5 scalar with the P-256 OID, then PEM (OpenSSL adds the public key).
printf '30310201010420c9afa9d845ba75166b5c215767b1d6934e50c3db36e89b127b8a622b120f6721a00a06082a8648ce3d030107' \
  | xxd -r -p > k.der
openssl ec -inform DER -in k.der -out rfc6979_a25_sec1.pem
openssl pkcs8 -topk8 -nocrypt -in rfc6979_a25_sec1.pem -out rfc6979_a25_pkcs8.pem
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-384 -out p384_pkcs8.pem
```

`openssl ec -in rfc6979_a25_pkcs8.pem -pubout -text -noout` prints the A.2.5 public key
(`Ux = 60FED4BA...60F29FB6`, `Uy = 7903FE10...D4462299`).

`keepassxc_style_entry.json` holds the PKCS#8 PEM above verbatim, credential id bytes `10 11 .. 1f`
and user handle `user-handle-1234`, base64url-encoded with and without padding, and the
SubjectPublicKeyInfo of the A.2.5 key (base64url) for verifying signatures.
