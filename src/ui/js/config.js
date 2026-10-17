// --- TRUSWAP Configuration & Global State ---
var BASE_CHAIN_ID = 8453; // Base Mainnet (84532 when --testnet)
var BASE_EXPLORER_URL = "https://basescan.org";
var BASE_NETWORK_NAME = "Base";
var IS_TESTNET = false;
var BASE_HTLC_ADDR = "0x812b611208297eb7A2b153BECA8b2AF47C7614DF";
var BASE_USDC_ADDR = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
var RELAY_NODE_URL = "wss://tru.powgrid.xyz/orderbook";

var ws = null;
var ordersMap = new Map();
var currentFilter = 'all';
var swapDirection = 'buy'; // 'buy' = Buy TRU, 'sell' = Sell TRU


var vaultData = {
  address: "",
  balance_atoms: 0,
  balance_tru: 0,
  confirmed_atoms: 0,
  confirmed_tru: 0,
  unconfirmed_atoms: 0,
  unconfirmed_tru: 0,
  base_address: "",
  base_eth_balance: 0,
  base_usdc_balance: 0,
  utxo_count: 0,
  utxos: []
};

var qrRenderedForAddr = null;
var exportKeysData = null;
var exportRevealed = { privhex: false, privwif: false, mnemonic: false, base_priv: false };
var mempoolData = { count: 0, txs: [], total_size: 0 };
var mempoolTimer = null;
var historicalLedgerData = null;


// --- DOM & String Utility Helpers ---
function setTextIfChanged(idOrEl, newText) {
  const el = (typeof idOrEl === 'string') ? document.getElementById(idOrEl) : idOrEl;
  if (!el || el.innerText === newText) return;
  el.innerText = newText;
}

function setHtmlIfChanged(idOrEl, newHtml) {
  const el = (typeof idOrEl === 'string') ? document.getElementById(idOrEl) : idOrEl;
  if (!el || el.innerHTML === newHtml) return;
  el.innerHTML = newHtml;
}

function updateTruBalanceDisplay(confirmedTru, unconfirmedTru) {
  const el = document.getElementById('tru-balance');
  if (!el) return;
  const c = Number(confirmedTru) || 0;
  const u = Number(unconfirmedTru) || 0;
  if (u > 0) {
    setHtmlIfChanged(el, `${c.toFixed(8)} <span style="font-size: 0.85rem; color: var(--tru-gold); font-weight: 600;">(+${u.toFixed(8)})</span>`);
  } else {
    setTextIfChanged(el, c.toFixed(8));
  }
}

let _toastTimer = null;
function showToast(msg, type = 'success') {
  const toast = document.getElementById('toast-msg');
  if (!toast) return;
  if (_toastTimer) clearTimeout(_toastTimer);
  toast.innerText = msg;
  toast.classList.remove('error', 'warning', 'info');
  if (type === 'error') {
    toast.classList.add('error');
  } else if (type === 'warning') {
    toast.classList.add('warning');
  } else if (type === 'info') {
    toast.classList.add('info');
  }
  toast.classList.add('show');
  const duration = (type === 'error' || type === 'warning') ? 4500 : 2800;
  _toastTimer = setTimeout(() => {
    toast.classList.remove('show');
    toast.classList.remove('error', 'warning', 'info');
  }, duration);
}

async function copyToClipboard(text, successMsg) {
  if (!text) return;
  
  // 0. Sync to native Android clipboard via WebSocket
  if (typeof sendSocketAction === 'function') {
    try {
      sendSocketAction('set_clipboard', { text });
    } catch (_) {}
  }

  // 1. Try modern Clipboard API if in secure context
  if (navigator.clipboard && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(text);
      showToast(successMsg);
      return;
    } catch (e) {
      console.warn("Clipboard API failed, falling back to execCommand:", e);
    }
  }

  // 2. Universal fallback for non-localhost HTTP / LAN IP
  try {
    const textArea = document.createElement("textarea");
    textArea.value = text;
    textArea.style.position = "fixed";
    textArea.style.left = "-9999px";
    textArea.style.top = "-9999px";
    textArea.setAttribute("readonly", "");
    document.body.appendChild(textArea);
    textArea.select();
    textArea.setSelectionRange(0, 99999);
    const successful = document.execCommand("copy");
    document.body.removeChild(textArea);
    if (successful) {
      showToast(successMsg);
      return;
    }
  } catch (err) {
    console.error("execCommand copy failed:", err);
  }

  // 3. Prompt fallback if all else fails
  window.prompt("Copy to clipboard: Ctrl+C, Enter", text);
}

function hexToBytes(hex) {
  let bytes = [];
  for (let c = 0; c < hex.length; c += 2)
    bytes.push(parseInt(hex.substr(c, 2), 16));
  return new Uint8Array(bytes);
}

// --- External URL Dispatcher (Safe Native System Browser Opening) ---
function isExternalUrl(url) {
  if (!url || typeof url !== 'string') return false;
  if (!url.startsWith('http://') && !url.startsWith('https://')) return false;
  try {
    const parsed = new URL(url, window.location.href);
    const host = parsed.hostname;
    const isLocal = host === '127.0.0.1' || host === 'localhost' || host === window.location.hostname;
    return !isLocal;
  } catch (_) {
    return false;
  }
}

function openExternalUrl(url) {
  if (!url) return;

  // 1. Dispatch over local WebSocket if active
  if (typeof sendSocketAction === 'function' && window.ws && window.ws.readyState === WebSocket.OPEN) {
    try {
      sendSocketAction('open_browser', { url }).catch(e => {
        console.warn("[ExternalUrl] WS open_browser error:", e);
      });
      return;
    } catch (_) {}
  }

  // 2. Dispatch over local REST endpoint
  try {
    fetch('/api/open_browser', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url: url })
    }).catch(e => {
      console.warn("[ExternalUrl] REST open_browser error:", e);
    });
  } catch (_) {}

  // 3. Fallback for non-Android browsers (e.g. desktop web browser)
  const isAndroid = navigator.userAgent.includes('TRUSWAP_APK') || /Android/i.test(navigator.userAgent);
  if (!isAndroid) {
    window.open(url, '_blank', 'noopener,noreferrer');
  }
}
window.openExternalUrl = openExternalUrl;

// Monkey-patch window.open to safeguard any dynamic script calls
const _nativeWindowOpen = window.open;
window.open = function(url, target, features) {
  if (isExternalUrl(url)) {
    openExternalUrl(url);
    return null;
  }
  return _nativeWindowOpen ? _nativeWindowOpen.call(window, url, target, features) : null;
};

// Global capture-phase click listener to intercept all <a> links to BaseScan and external sites
document.addEventListener('click', function(e) {
  const link = e.target.closest('a');
  if (!link) return;

  const rawHref = link.getAttribute('href');
  if (!rawHref || rawHref.startsWith('#') || rawHref.startsWith('javascript:')) {
    return;
  }

  const href = link.href;
  if (isExternalUrl(href)) {
    e.preventDefault();
    e.stopPropagation();
    openExternalUrl(href);
  }
}, true);

