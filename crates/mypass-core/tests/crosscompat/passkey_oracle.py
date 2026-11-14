"""Independent oracle for the passkey ceremonies and the vault fields (CI only, never shipped).

Usage: passkey_oracle.py <dir>

<dir> holds `vectors.json` and `vault.kdbx` written by
`cargo run -p mypass-core --example write_passkey_vectors -- <dir>`. This script only VERIFIES our
output with other implementations (Yubico python-fido2, pykeepass, cryptography); it never produces
the known-answer values pinned in `tests/passkey_vectors.rs`. Every check prints one line; any
failure exits non-zero. It also writes `<dir>/pykeepass/` (a vault made by pykeepass with
KeePassXC-style passkey entries plus `expected.json`), which the env-gated Rust test
`tests/passkey_pykeepass.rs` reads back.
"""
import base64
import json
import re
import sys
from pathlib import Path

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import ec
from fido2.attestation import Attestation
from fido2.server import Fido2Server
from fido2.utils import websafe_decode, websafe_encode
from fido2.webauthn import AttestationObject, AttestedCredentialData

from pykeepass import PyKeePass, create_database

AAGUID = bytes.fromhex("94803471f8b448f998fffc4ac1a92af8")
PREFIX = "KPEX_PASSKEY_"
EIGHT = [
    "RELYING_PARTY", "CREDENTIAL_ID", "PRIVATE_KEY_PEM", "USER_HANDLE",
    "USERNAME", "FLAG_BE", "FLAG_BS", "CREATED",
]
PROTECTED = {"CREDENTIAL_ID", "PRIVATE_KEY_PEM", "USER_HANDLE"}
FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "passkey" / "keepassxc_style_entry.json"


def ok(message):
    print(f"ok: {message}")


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def b64(value):
    return websafe_decode(value)


def public_numbers(cose):
    # COSE EC2 key: -2 is x, -3 is y.
    return int.from_bytes(cose[-2], "big"), int.from_bytes(cose[-3], "big")


def check_registration(vectors, case, state_uv="preferred"):
    request = json.loads(case["create_request"])
    packed = request["attestation"] in ("direct", "indirect", "enterprise")
    server = Fido2Server(
        {"id": vectors["rp_id"], "name": "Example"},
        attestation="direct" if packed else "none",
        verify_origin=lambda origin: origin == vectors["origin"],
        verify_attestation=(
            (lambda att, h: _verify_self_attestation(att, h)) if packed else None
        ),
    )
    state = {"challenge": request["challenge"], "user_verification": state_uv}
    auth_data = server.register_complete(state, case["create_response"])
    cred = auth_data.credential_data
    require(cred is not None, "no attested credential data")
    require(cred.aaguid == AAGUID, "unexpected AAGUID")
    require(cred.credential_id == b64(case["credential_id"]), "credential id mismatch")
    require(auth_data.is_user_present() and auth_data.is_user_verified(), "UP/UV not set")
    require(auth_data.flags & 0x08 and auth_data.flags & 0x10, "BE/BS not set on create")
    require(auth_data.counter == 0, "counter is not 0")
    require(dict(cred.public_key)[3] == -7, "not an ES256 key")
    fmt = "packed" if packed else "none"
    # The attestation format actually present in the object must match the request.
    att_object = case["create_response"]["response"]["attestationObject"]
    require(AttestationObject(b64(att_object)).fmt == fmt, f"attestation format is not {fmt}")
    ok(f"register_complete accepts {case['name']} (attestation {fmt}, origin {vectors['origin']})")
    return cred


def _verify_self_attestation(attestation_object, client_data_hash):
    result = Attestation.for_type(attestation_object.fmt)().verify(
        attestation_object.att_stmt, attestation_object.auth_data, client_data_hash
    )
    require(result.trust_path == [], "self attestation must carry no x5c chain")


def check_assertions(vectors, case, cred):
    server = Fido2Server(
        {"id": vectors["rp_id"], "name": "Example"},
        verify_origin=lambda origin: origin == vectors["origin"],
    )
    for index, assertion in enumerate(case["assertions"], 1):
        request = json.loads(assertion["request"])
        state = {"challenge": request["challenge"], "user_verification": "preferred"}
        found = server.authenticate_complete(state, [cred], assertion["response"])
        require(found.credential_id == cred.credential_id, "wrong credential returned")
        response = assertion["response"]["response"]
        require(b64(response["userHandle"]) == b64(case["user_handle"]), "user handle mismatch")
        ok(f"authenticate_complete accepts assertion {index} of {case['name']}")
    # A tampered signature must be rejected (the oracle is not a rubber stamp).
    bad = json.loads(json.dumps(case["assertions"][0]["response"]))
    sig = bytearray(b64(bad["response"]["signature"]))
    sig[-1] ^= 0x01
    bad["response"]["signature"] = websafe_encode(bytes(sig))
    state = {"challenge": json.loads(case["assertions"][0]["request"])["challenge"],
             "user_verification": "preferred"}
    try:
        server.authenticate_complete(state, [cred], bad)
    except Exception:
        ok(f"tampered signature of {case['name']} is rejected")
    else:
        raise AssertionError("a tampered signature was accepted")


def check_vault(vectors, creds):
    vault = PyKeePass(str(Path(sys.argv[1]) / vectors["vault"]), password=vectors["vault_password"])
    for case in vectors["cases"]:
        entry = vault.find_entries(title=case["name"], first=True)
        require(entry is not None, f"entry {case['name']} not found by pykeepass")
        props = {k: v for k, v in entry.custom_properties.items() if k.startswith(PREFIX)}
        require(sorted(props) == sorted(PREFIX + k for k in EIGHT),
                f"{case['name']}: unexpected KPEX properties {sorted(props)}")
        require(props[PREFIX + "RELYING_PARTY"] == vectors["rp_id"], "RELYING_PARTY")
        require(b64(props[PREFIX + "CREDENTIAL_ID"]) == b64(case["credential_id"]), "CREDENTIAL_ID")
        require(b64(props[PREFIX + "USER_HANDLE"]) == b64(case["user_handle"]), "USER_HANDLE")
        require(props[PREFIX + "USERNAME"] == case["username"], "USERNAME")
        require(props[PREFIX + "FLAG_BE"] in ("0", "1") and props[PREFIX + "FLAG_BS"] in ("0", "1"),
                "flag shape")
        require(re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", props[PREFIX + "CREATED"]),
                "CREATED shape")
        require(props[PREFIX + "CREATED"] == vectors["created"], "CREATED value")
        for key in PROTECTED:
            strings = entry._element.xpath(f'String[Key="{PREFIX}{key}"]/Value')
            require(strings and strings[0].get("Protected") == "True",
                    f"{PREFIX}{key} is not a protected value")
        pem = props[PREFIX + "PRIVATE_KEY_PEM"]
        require(pem.startswith("-----BEGIN PRIVATE KEY-----"), "PEM is not PKCS#8")
        key = serialization.load_pem_private_key(pem.encode(), password=None)
        require(isinstance(key, ec.EllipticCurvePrivateKey) and key.curve.name == "secp256r1",
                "PEM is not a P-256 key")
        numbers = key.public_key().public_numbers()
        require((numbers.x, numbers.y) == public_numbers(creds[case["name"]].public_key),
                "PEM public key does not match the registered credential")
        ok(f"pykeepass reads {case['name']}: 8 KPEX properties, protected, PKCS#8 P-256 key matches credential")


def write_keepassxc_style(out):
    """Reverse direction: pykeepass + cryptography write KeePassXC-shaped passkey entries."""
    fixture = json.loads(FIXTURE.read_text())
    out.mkdir(parents=True, exist_ok=True)
    path = out / "vault.kdbx"
    vault = create_database(str(path), password="fixture-password")
    expected = []
    for name, entry in fixture["entries"].items():
        fields = {f["key"]: (f["value"], f["protected"]) for f in entry["fields"]}
        e = vault.add_entry(vault.root_group, name, fields["UserName"][0], "")
        e.url = fields["URL"][0]
        for key, (value, protect) in fields.items():
            if key.startswith(PREFIX):
                e.set_custom_property(key, value, protect=protect)
        expected.append({"title": name, "rp_id": fields[PREFIX + "RELYING_PARTY"][0],
                         "credential_id": fixture["credential_id_hex"],
                         "user_handle": fixture["user_handle_hex"],
                         "public_key_spki": fixture["public_key_spki_der_b64url"]})
    # A key generated by `cryptography` (its own PKCS#8 encoding), id padded and handle standard-base64.
    key = ec.generate_private_key(ec.SECP256R1())
    pem = key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                            serialization.NoEncryption()).decode()
    spki = key.public_key().public_bytes(serialization.Encoding.DER,
                                         serialization.PublicFormat.SubjectPublicKeyInfo)
    cred_id = bytes(range(100, 116))
    handle = b"\xfb\xff\xfeuser"  # standard base64 would use '+' and '/'
    e = vault.add_entry(vault.root_group, "cryptography-key", "bob@example.org", "")
    for key_name, value, protect in [
        ("RELYING_PARTY", "example.org", False),
        ("CREDENTIAL_ID", base64.urlsafe_b64encode(cred_id).decode(), True),
        ("PRIVATE_KEY_PEM", pem, True),
        ("USER_HANDLE", base64.b64encode(handle).decode(), True),
        ("USERNAME", "bob@example.org", False),
    ]:
        e.set_custom_property(PREFIX + key_name, value, protect=protect)
    expected.append({"title": "cryptography-key", "rp_id": "example.org",
                     "credential_id": cred_id.hex(), "user_handle": handle.hex(),
                     "public_key_spki": websafe_encode(spki)})
    vault.save()
    (out / "expected.json").write_text(json.dumps({"password": "fixture-password", "entries": expected}))
    ok(f"pykeepass wrote {len(expected)} KeePassXC-style passkey entries for the Rust reader")


def main():
    directory = Path(sys.argv[1])
    vectors = json.loads((directory / "vectors.json").read_text())
    creds = {}
    for case in vectors["cases"]:
        cred = check_registration(vectors, case)
        # The registered credential must round-trip through python-fido2's own encoding.
        require(AttestedCredentialData(bytes(cred)) == cred, "credential data does not round-trip")
        creds[case["name"]] = cred
        check_assertions(vectors, case, cred)
    check_vault(vectors, creds)
    write_keepassxc_style(directory / "pykeepass")
    print("PASSKEY_ORACLE_OK")


if __name__ == "__main__":
    try:
        if len(sys.argv) != 2:
            raise ValueError("usage: passkey_oracle.py <dir>")
        main()
    except Exception as error:
        sys.exit(f"PASSKEY_ORACLE_FAILED: {type(error).__name__}: {error}")
