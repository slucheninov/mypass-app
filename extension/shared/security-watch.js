// Remembers a login MyPass just filled and decides when a later paste or copy
// is happening on a site that is not saved with that item. The service worker
// stores the secrets; this file only decides. A page never sees the result
// except as a yes/no warning.
//
// One var bag. A top-level const or function here would share the classic-script
// scope with the other extension scripts and can throw at load.

var mypassSecurityWatch = (function () {
  var SECURITY_SETTINGS_KEY = 'mypassSecuritySettings';
  var SECURITY_WATCH_KEY = 'mypassSecurityWatch';
  var SECURITY_WATCH_TTL_MS = 5 * 60 * 1000;
  var SECURITY_WATCH_LIMIT = 8;
  var SECURITY_MATCH_MIN_LENGTH = 4;
  var SECURITY_TEXT_MAX_LENGTH = 500;

  function defaultSecuritySettings() {
    return { warnPaste: true, warnCopy: true };
  }

  function normalizeSecuritySettings(value) {
    var defaults = defaultSecuritySettings();
    if (!value || typeof value !== 'object') return defaults;
    return {
      warnPaste: typeof value.warnPaste === 'boolean' ? value.warnPaste : defaults.warnPaste,
      warnCopy: typeof value.warnCopy === 'boolean' ? value.warnCopy : defaults.warnCopy
    };
  }

  function securityHost(url) {
    try {
      return new URL(url).hostname.toLowerCase();
    } catch (e) {
      return '';
    }
  }

  function hostsForFill(requestUrl, extraUrls) {
    var hosts = [];
    function add(url) {
      var host = securityHost(url);
      if (host && hosts.indexOf(host) === -1) hosts.push(host);
    }
    add(requestUrl);
    if (Array.isArray(extraUrls)) {
      for (var i = 0; i < extraUrls.length; i++) add(extraUrls[i]);
    }
    return hosts;
  }

  // www.bank.example and bank.example are the same saved site. A different
  // registrable name is not: evil.example does not cover bank.example.
  function hostIsCovered(pageHost, savedHosts) {
    if (!pageHost || !Array.isArray(savedHosts)) return false;
    for (var i = 0; i < savedHosts.length; i++) {
      var saved = savedHosts[i];
      if (!saved) continue;
      if (pageHost === saved || pageHost.endsWith('.' + saved) || saved.endsWith('.' + pageHost)) return true;
    }
    return false;
  }

  function isLiveWatch(watch, now) {
    return !!(watch && typeof watch.at === 'number' && now - watch.at <= SECURITY_WATCH_TTL_MS && Array.isArray(watch.hosts));
  }

  function copyWatch(watch, at) {
    return {
      username: typeof watch.username === 'string' ? watch.username : '',
      password: typeof watch.password === 'string' ? watch.password : '',
      hosts: watch.hosts.slice(),
      at: at
    };
  }

  function rememberFilledCredential(watches, credential, requestUrl, now) {
    var list = [];
    var source = Array.isArray(watches) ? watches : [];
    for (var i = 0; i < source.length; i++) {
      if (isLiveWatch(source[i], now)) list.push(copyWatch(source[i], source[i].at));
    }
    if (!credential || credential.type !== 'credential') return list;
    var password = typeof credential.password === 'string' ? credential.password : '';
    var username = typeof credential.username === 'string' ? credential.username : '';
    if (password.length < SECURITY_MATCH_MIN_LENGTH && username.length < SECURITY_MATCH_MIN_LENGTH) return list;
    var hosts = hostsForFill(requestUrl, credential.urls);
    if (!hosts.length) return list;
    for (var j = 0; j < list.length; j++) {
      if (list[j].password === password && list[j].username === username) {
        for (var h = 0; h < hosts.length; h++) {
          if (list[j].hosts.indexOf(hosts[h]) === -1) list[j].hosts.push(hosts[h]);
        }
        list[j].at = now;
        return list.slice(-SECURITY_WATCH_LIMIT);
      }
    }
    list.push({ username: username, password: password, hosts: hosts, at: now });
    return list.slice(-SECURITY_WATCH_LIMIT);
  }

  function evaluateSecurityUse(settings, watches, action, text, pageUrl, now) {
    var normalized = normalizeSecuritySettings(settings);
    var active = [];
    var source = Array.isArray(watches) ? watches : [];
    for (var i = 0; i < source.length; i++) {
      if (isLiveWatch(source[i], now)) active.push(copyWatch(source[i], source[i].at));
    }
    var value = typeof text === 'string' ? text : '';
    if (value.length > SECURITY_TEXT_MAX_LENGTH) value = value.slice(0, SECURITY_TEXT_MAX_LENGTH);
    var host = securityHost(pageUrl);
    if ((action !== 'paste' && action !== 'copy') || value.length < SECURITY_MATCH_MIN_LENGTH || !host) {
      return { warn: false, watches: active };
    }
    var fields = action === 'paste' ? ['password'] : ['username', 'password'];
    var related = [];
    for (var w = 0; w < active.length; w++) {
      for (var f = 0; f < fields.length; f++) {
        var field = active[w][fields[f]];
        if (field && field === value) {
          related.push(active[w]);
          break;
        }
      }
    }
    if (!related.length) return { warn: false, watches: active };
    for (var r = 0; r < related.length; r++) {
      if (hostIsCovered(host, related[r].hosts)) {
        for (var k = 0; k < related.length; k++) related[k].at = now;
        return { warn: false, watches: active };
      }
    }
    var enabled = action === 'paste' ? normalized.warnPaste : normalized.warnCopy;
    return { warn: enabled, watches: active };
  }

  function textFromCopyTarget(value, selectionStart, selectionEnd, selectionText) {
    if (typeof value === 'string' && typeof selectionStart === 'number' && typeof selectionEnd === 'number' && selectionStart !== selectionEnd) {
      return value.slice(Math.max(0, selectionStart), Math.max(0, selectionEnd));
    }
    return typeof selectionText === 'string' ? selectionText : '';
  }

  return {
    SECURITY_SETTINGS_KEY: SECURITY_SETTINGS_KEY,
    SECURITY_WATCH_KEY: SECURITY_WATCH_KEY,
    SECURITY_WATCH_TTL_MS: SECURITY_WATCH_TTL_MS,
    SECURITY_MATCH_MIN_LENGTH: SECURITY_MATCH_MIN_LENGTH,
    defaultSecuritySettings: defaultSecuritySettings,
    normalizeSecuritySettings: normalizeSecuritySettings,
    rememberFilledCredential: rememberFilledCredential,
    evaluateSecurityUse: evaluateSecurityUse,
    textFromCopyTarget: textFromCopyTarget
  };
})();

if (typeof module !== 'undefined') {
  module.exports = mypassSecurityWatch;
}
