#!/usr/bin/env bash
# Generates the corpus's signature fixtures and the test-only public keys they verify against.
# Every key is made here, in a throwaway GNUPGHOME, and its secret half is discarded on exit:
# none of them is anyone's real key. Rerunning rewrites every file with new keys and signatures.
#
# Usage: crates/postio-model/tests/corpus/keys/generate.sh
set -euo pipefail

KEYS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORPUS="$(dirname "$KEYS")"
export GNUPGHOME
GNUPGHOME="$(mktemp -d)"
trap 'gpgconf --kill all >/dev/null 2>&1 || true; rm -rf "$GNUPGHOME"' EXIT

KEY_TIME=20260303T090000
SIGN_TIME=20260305T090000

g() { gpg --batch --quiet --no-tty --pinentry-mode loopback --passphrase '' "$@"; }
at() { local time=$1; shift; g --faked-system-time "${time}!" "$@"; }
fpr() { g --with-colons --fingerprint "$1" | awk -F: '/^fpr/ {print $10; exit}'; }
crlf() { sed 's/\r$//; s/$/\r/'; }

at "$KEY_TIME" --quick-generate-key 'Ada Moreno (corpus key A) <ada@example.com>' ed25519 sign never
at "$KEY_TIME" --quick-generate-key 'Ada Moreno (corpus key B) <ada@example.com>' rsa3072 sign never
at "$KEY_TIME" --quick-generate-key 'Lin Okafor (corpus key, never exported) <lin@example.org>' ed25519 sign never
A=$(fpr 'corpus key A')
B=$(fpr 'corpus key B')
C=$(fpr 'never exported')

g --armor --export "$A" > "$KEYS/ada-a.asc"
g --armor --export "$B" > "$KEYS/ada-b.asc"

# A detached signature over stdin's bytes, exactly as given.
detach() { local key=$1 digest=$2; at "$SIGN_TIME" --local-user "$key" --digest-algo "$digest" --armor --detach-sign; }

# multipart/signed around a signed part already in CRLF form (RFC 3156: the CRLF before the
# boundary belongs to the boundary, so the part's last CRLF is not signed).
signed_message() {
    local headers=$1 part=$2 key=$3 digest=$4 micalg=$5 boundary=$6
    local signature
    signature=$(printf '%s' "$part" | detach "$key" "$digest" | crlf)
    printf '%s\n' "$headers"
    printf 'MIME-Version: 1.0\r\n'
    printf 'Content-Type: multipart/signed; micalg=%s;\r\n' "$micalg"
    printf '\tprotocol="application/pgp-signature"; boundary="%s"\r\n\r\n' "$boundary"
    printf 'This is an OpenPGP/MIME signed message (RFC 3156).\r\n'
    printf -- '--%s\r\n%s\r\n--%s\r\n' "$boundary" "$part" "$boundary"
    printf 'Content-Type: application/pgp-signature; name="signature.asc"\r\n'
    printf 'Content-Description: OpenPGP digital signature\r\n'
    printf 'Content-Disposition: attachment; filename="signature.asc"\r\n\r\n'
    printf '%s\n' "$signature"
    printf -- '--%s--\r\n' "$boundary"
}

envelope() {
    local from=$1 id=$2 subject=$3
    printf 'Message-ID: <%s@example.com>\r\n' "$id"
    printf 'Date: Thu, 05 Mar 2026 09:00:00 +0000\r\n'
    printf 'From: %s\r\n' "$from"
    printf 'To: Grace Example <grace@example.net>\r\n'
    printf 'Subject: %s\r\n' "$subject"
}

ADA='Ada Moreno <ada@example.com>'

PLAIN_PART=$(crlf <<'EOF' | sed '$ s/\r$//'
Content-Type: text/plain; charset=utf-8
Content-Transfer-Encoding: 7bit

The lock-keeper rota for April, signed so it is on the record.

  Week 1  Ada
  Week 2  Grace
  Week 3  Ada
  Week 4  Lin

Swaps go through me, not the shared sheet.

Ada
EOF
)

signed_message "$(envelope "$ADA" pgp-signed-good 'Signed: lock-keeper rota, April')" \
    "$PLAIN_PART" "$A" SHA256 pgp-sha256 '=_sig_good_7d1' > "$CORPUS/pgp-signed-good.eml"

sed 's/  Week 4  Lin/  Week 4  Kim/' "$CORPUS/pgp-signed-good.eml" \
    | sed 's/pgp-signed-good@/pgp-signed-tampered-body@/; s/^Subject: Signed:/Subject: Tampered body:/' \
    > "$CORPUS/pgp-signed-tampered-body.eml"

sed 's/^Content-Transfer-Encoding: 7bit\r$/Content-Transfer-Encoding: 8bit\r/' "$CORPUS/pgp-signed-good.eml" \
    | sed 's/pgp-signed-good@/pgp-signed-tampered-header@/; s/^Subject: Signed:/Subject: Tampered header:/' \
    > "$CORPUS/pgp-signed-tampered-header.eml"

QP_PART=$(crlf <<'EOF' | sed '$ s/\r$//'
Content-Type: text/plain; charset=utf-8
Content-Transfer-Encoding: quoted-printable

Caf=C3=A9 order for the lock-keepers' meeting, signed as agreed. Totals: 3=
 flat whites =3D 9.60, 2 teas =3D 4.00.

The line above is soft-broken on purpose: the signature covers the encoded
form, so a client that decodes and re-encodes it breaks the signature.

Ada
EOF
)

signed_message "$(envelope "$ADA" pgp-signed-qp 'Signed: café order')" \
    "$QP_PART" "$A" SHA256 pgp-sha256 '=_sig_qp_51c' > "$CORPUS/pgp-signed-qp.eml"

QP_DECODED=$'Café order for the lock-keepers\' meeting, signed as agreed. Totals: 3 flat whites = 9.60, 2 teas = 4.00.\n\nThe line above is soft-broken on purpose: the signature covers the encoded\nform, so a client that decodes and re-encodes it breaks the signature.\n\nAda\n'
REENCODED_PART=$( {
    printf 'Content-Type: text/plain; charset=utf-8\r\n'
    printf 'Content-Transfer-Encoding: base64\r\n\r\n'
    printf '%s' "$QP_DECODED" | crlf | base64 -w 76 | crlf
} | sed '$ s/\r$//')
# The quoted-printable fixture's signature, moved onto the same text re-encoded as base64.
QP_SIGNATURE=$(awk '/BEGIN PGP SIGNATURE/,/END PGP SIGNATURE/' "$CORPUS/pgp-signed-qp.eml")
{
    envelope "$ADA" pgp-signed-reencoded 'Re-encoded: café order'
    printf 'MIME-Version: 1.0\r\n'
    printf 'Content-Type: multipart/signed; micalg=pgp-sha256;\r\n'
    printf '\tprotocol="application/pgp-signature"; boundary="=_sig_reenc_90a"\r\n\r\n'
    printf -- '--=_sig_reenc_90a\r\n%s\r\n--=_sig_reenc_90a\r\n' "$REENCODED_PART"
    printf 'Content-Type: application/pgp-signature; name="signature.asc"\r\n\r\n'
    printf '%s\n' "$QP_SIGNATURE"
    printf -- '--=_sig_reenc_90a--\r\n'
} > "$CORPUS/pgp-signed-reencoded.eml"

LIN_PART=$(crlf <<'EOF' | sed '$ s/\r$//'
Content-Type: text/plain; charset=utf-8

Signed by a key that is on no computer but the one that made it.

Lin
EOF
)
signed_message "$(envelope 'Lin Okafor <lin@example.org>' pgp-signed-no-key 'Signed: towpath closure')" \
    "$LIN_PART" "$C" SHA256 pgp-sha256 '=_sig_nokey_3e2' > "$CORPUS/pgp-signed-no-key.eml"

signed_message "$(envelope "$ADA" pgp-signed-other-key 'Signed: lock-keeper rota, with my other key')" \
    "$PLAIN_PART" "$B" SHA512 pgp-sha512 '=_sig_other_c40' > "$CORPUS/pgp-signed-other-key.eml"

# Key B, because GnuPG refuses SHA-1 with an Ed25519 key.
signed_message "$(envelope "$ADA" pgp-signed-sha1 'Signed with SHA-1: lock-keeper rota')" \
    "$PLAIN_PART" "$B" SHA1 pgp-sha1 '=_sig_sha1_e05' > "$CORPUS/pgp-signed-sha1.eml"

{
    envelope "$ADA" pgp-inline-signed 'Inline signed: sluice inspection'
    printf 'MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n'
    printf 'Content-Transfer-Encoding: 7bit\r\n\r\n'
    printf '%s\n' 'The sluice inspection is moved to Friday.' '' \
        '- This line starts with a dash, so the armour escapes it.' '' 'Ada' \
        | at "$SIGN_TIME" --local-user "$A" --digest-algo SHA256 --clearsign | crlf
} > "$CORPUS/pgp-inline-signed.eml"

LIST_SIGNED=$(signed_message '' "$PLAIN_PART" "$A" SHA256 pgp-sha256 '=_sig_inner_b8f' \
    | sed '1,/^MIME-Version/ d' | sed '$ s/\r$//')
{
    envelope "$ADA" pgp-signed-list-footer '[lock-keepers] Signed: lock-keeper rota, April'
    printf 'List-Id: Lock keepers <lock-keepers.lists.example.org>\r\n'
    printf 'MIME-Version: 1.0\r\n'
    printf 'Content-Type: multipart/mixed; boundary="=_list_wrap_19c"\r\n\r\n'
    printf -- '--=_list_wrap_19c\r\n%s\r\n--=_list_wrap_19c\r\n' "$LIST_SIGNED"
    printf 'Content-Type: text/plain; charset=utf-8\r\n\r\n'
    printf 'lock-keepers mailing list\r\nUnsubscribe: https://lists.example.org/lock-keepers\r\n'
    printf -- '--=_list_wrap_19c--\r\n'
} > "$CORPUS/pgp-signed-list-footer.eml"

{
    envelope "$ADA" autocrypt-header 'Moorings for the summer'
    printf 'Autocrypt: addr=ada@example.com; prefer-encrypt=mutual; keydata=\r\n'
    g --export --export-options export-minimal "$A" | base64 -w 76 | sed 's/^/ /' | crlf
    printf 'MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n'
    printf 'Not signed: the Autocrypt header only carries my key, for next time.\r\n\r\nAda\r\n'
} > "$CORPUS/autocrypt-header.eml"

{
    envelope "$ADA" pgp-key-attached 'My key, attached'
    printf 'MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary="=_key_att_6a0"\r\n\r\n'
    printf -- '--=_key_att_6a0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n'
    printf 'My public key is attached, if you want to check what I sign.\r\n\r\nAda\r\n'
    printf -- '--=_key_att_6a0\r\nContent-Type: application/pgp-keys; name="ada.asc"\r\n'
    printf 'Content-Disposition: attachment; filename="ada.asc"\r\n\r\n'
    crlf < "$KEYS/ada-a.asc"
    printf -- '--=_key_att_6a0--\r\n'
} > "$CORPUS/pgp-key-attached.eml"

echo "key A $A"
echo "key B $B"
echo "wrote $(ls "$CORPUS"/pgp-*.eml "$CORPUS"/autocrypt-header.eml | wc -l) fixtures"
