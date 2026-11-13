// Hash router for the options page (chrome-extension://<id>/app/app.html#/...).
// Chrome opens the standard Options entry without a hash, so the empty hash is
// treated as the default route and the page rewrites it to the canonical form.

var mypassRouter = (function () {
  var DEFAULT_ROUTE = '/page/settings';

  function routePath(hash) {
    var value = String(hash || '');
    if (value.charAt(0) === '#') value = value.slice(1);
    return value === '' || value === '/' ? DEFAULT_ROUTE : value;
  }

  function parseRoute(hash) {
    if (routePath(hash) === '/page/settings') return { name: 'settings' };
    return { name: 'notFound' };
  }

  // Settings-equivalent input maps to the canonical hash; anything else is
  // returned unchanged so unknown routes are not silently rewritten.
  function canonicalHash(hash) {
    var route = parseRoute(hash);
    if (route.name === 'settings') return '#' + DEFAULT_ROUTE;
    return String(hash || '');
  }

  return {
    DEFAULT_ROUTE: DEFAULT_ROUTE,
    parseRoute: parseRoute,
    canonicalHash: canonicalHash
  };
})();

if (typeof window !== 'undefined') {
  window.mypassRouter = mypassRouter;
}

if (typeof module !== 'undefined') {
  module.exports = mypassRouter;
}
