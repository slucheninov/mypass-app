// Base32 check for TOTP setup keys (RFC 4648 alphabet, no padding).
// The page scanner and the popup paste field both accept a secret only
// through this function. Whitespace is stripped and case is folded before
// the length check, so "jbsw y3dp ehpk 3pxp" and the uppercase key are the
// same secret. 16 is the shortest key this extension will store; 128 is the
// cap that rejects a paragraph of uppercase words.

const BASE32_REGEX = /^[A-Z2-7]{16,128}$/;

function normalizeBase32Candidate(raw) {
  return (raw || '').replace(/\s+/g, '').toUpperCase();
}

function isBase32Secret(candidate) {
  return BASE32_REGEX.test(normalizeBase32Candidate(candidate));
}

var mypassBase32 = { normalizeBase32Candidate, isBase32Secret };

if (typeof module !== 'undefined') {
  module.exports = { BASE32_REGEX, normalizeBase32Candidate, isBase32Secret };
}
