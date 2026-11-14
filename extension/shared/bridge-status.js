// How a getCredentials response is classified before any UI branches on it.
// The popup and the in-page panel both used to keep this switch. A new
// response type added to only one of them routed the other surface wrong
// (a locked vault treated as "no logins", a pairing prompt treated as an error).

function classifyGetCredentialsResponse(response) {
  if (!response || response.type === 'error') return 'error';
  if (response.type === 'locked') return 'locked';
  if (response.type === 'pairingRequired') return 'pairingRequired';
  if (response.type === 'credentialList') return 'credentialList';
  return 'unknown';
}

// Connection failures do not establish whether the app is installed or running.
// Only the background worker's explicit Chrome missing-host code indicates setup.
function classifyBridgeConnectionError(response) {
  return response && response.code === 'native_host_not_found'
    ? 'native_host_not_found' : 'connection_unavailable';
}

var mypassBridgeStatus = { classifyGetCredentialsResponse, classifyBridgeConnectionError };

if (typeof module !== 'undefined') {
  module.exports = mypassBridgeStatus;
}
