#!/usr/bin/env bash
# Generates the corpus's S/MIME fixtures and the test-only certificates they chain to. Every key
# is made here, in a throwaway directory, and every private key is discarded on exit: only the
# two roots' certificates are kept, as DER, the anchors a test hands the verifier.
#
# Validity is fixed (2026-01-01 to 2036-01-01; the expired leaf ends in 2021), so a test checks
# at a fixed time. Rerunning rewrites every S/MIME fixture with new keys.
#
# Usage: crates/postio-model/tests/corpus/keys/smime/generate.sh
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORPUS="$(dirname "$(dirname "$HERE")")"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

FROM=20260101000000Z
UNTIL=20360101000000Z

ca() {
    openssl req -x509 -newkey rsa:2048 -nodes -keyout "$1.key" -out "$1.pem" -subj "/CN=$2" \
        -not_before "$FROM" -not_after "$UNTIL" \
        -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign,cRLSign 2>/dev/null
}
leaf() { # name email eku signer not_before not_after
    openssl req -newkey rsa:2048 -nodes -keyout "$1.key" -out "$1.csr" -subj "/CN=$2" 2>/dev/null
    printf 'basicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=%s\nsubjectAltName=email:%s\n' \
        "$3" "$2" > "$1.ext"
    openssl x509 -req -in "$1.csr" -CA "$4.pem" -CAkey "$4.key" -CAcreateserial -out "$1.pem" \
        -not_before "$5" -not_after "$6" -extfile "$1.ext" 2>/dev/null
}

ca root "Corpus Test Root"
ca stranger "Corpus Untrusted Root"
openssl req -newkey rsa:2048 -nodes -keyout inter.key -out inter.csr -subj "/CN=Corpus Test Intermediate" 2>/dev/null
printf 'basicConstraints=critical,CA:TRUE,pathlen:0\nkeyUsage=critical,keyCertSign,cRLSign\n' > inter.ext
openssl x509 -req -in inter.csr -CA root.pem -CAkey root.key -CAcreateserial -out inter.pem \
    -not_before "$FROM" -not_after "$UNTIL" -extfile inter.ext 2>/dev/null
leaf good ada@example.com emailProtection inter "$FROM" "$UNTIL"
leaf expired ada@example.com emailProtection inter 20200101000000Z 20210101000000Z
leaf unknown ada@example.com emailProtection stranger "$FROM" "$UNTIL"
leaf mismatch grace@example.net emailProtection inter "$FROM" "$UNTIL"

openssl x509 -in root.pem -outform DER -out "$HERE/root.der"
openssl x509 -in stranger.pem -outform DER -out "$HERE/stranger.der"

crlf() { sed 's/\r$//; s/$/\r/'; }
printf 'Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nThe harbour rota for March is attached to the noticeboard.\r\n\r\nAda\r\n' > part.txt

fixture() { # name signer chain subject [tamper]
    local name=$1 signer=$2 chain=$3 subject=$4 tamper=${5:-}
    local signed
    signed=$(openssl cms -sign -md sha256 -in part.txt -signer "$signer.pem" -inkey "$signer.key" \
        -certfile "$chain.pem" -outform SMIME | crlf)
    {
        printf 'Message-ID: <%s@example.com>\r\n' "$name"
        printf 'Date: Thu, 05 Mar 2026 09:00:00 +0000\r\n'
        printf 'From: Ada Moreno <ada@example.com>\r\n'
        printf 'To: Grace Example <grace@example.net>\r\n'
        printf 'Subject: %s\r\n' "$subject"
        if [ -n "$tamper" ]; then
            printf '%s\n' "$signed" | sed 's/The harbour rota for March/The harbour rota for April/'
        else
            printf '%s\n' "$signed"
        fi
    } > "$CORPUS/$name.eml"
}

fixture smime-signed-good good inter 'Harbour rota'
fixture smime-signed-tampered good inter 'Harbour rota' tamper
fixture smime-signed-expired expired inter 'Harbour rota'
fixture smime-signed-unknown-anchor unknown stranger 'Harbour rota'
fixture smime-signed-address-mismatch mismatch inter 'Harbour rota'

echo "wrote $(ls "$CORPUS"/smime-signed-*.eml | wc -l) S/MIME fixtures"
