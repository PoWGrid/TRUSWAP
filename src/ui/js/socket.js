// --- WebSocket Client & Event Dispatcher ---
let latestBlockHeight = 0;
let latestPeerCount = 0;
let latestVerifiedHeight = 0;
let latestTargetHeight = 0;
let latestSwapCount = 0;

function updateHeaderChainDisplay() {
  const dotScanner = document.getElementById('dot-scanner');
  const statusEl = document.getElementById('header-chain-status');
  const peersEl = document.getElementById('header-peers-badge');
  if (!statusEl) return;

  const isSyncing = latestTargetHeight > 0 && latestVerifiedHeight > 0 && latestVerifiedHeight < latestTargetHeight;
  const currentTip = isSyncing ? latestVerifiedHeight : Math.max(latestVerifiedHeight, latestTargetHeight, latestBlockHeight);

  if (currentTip > 0) {
    latestBlockHeight = Math.max(latestBlockHeight, currentTip);
    refreshUtxoConfirmations();
  }

  if (peersEl) {
    setTextIfChanged(peersEl, `${latestPeerCount} ${latestPeerCount === 1 ? 'peer' : 'peers'}`);
  }

  const finalTip = currentTip > 0 ? currentTip : latestBlockHeight;
  if (finalTip > 0) {
    setTextIfChanged(statusEl, `#${finalTip.toLocaleString()}`);
  } else {
    setTextIfChanged(statusEl, `#...`);
  }

  const scannerPill = document.getElementById('pill-scanner');
  if (isSyncing) {
    const pct = ((latestVerifiedHeight / latestTargetHeight) * 100).toFixed(1);
    if (scannerPill) {
      scannerPill.title = `TRU Blockchain & Ledger • Syncing ${pct}% (#${latestVerifiedHeight.toLocaleString()} / #${latestTargetHeight.toLocaleString()})`;
    }
    if (dotScanner && dotScanner.className !== 'chain-peer-dot syncing') {
      dotScanner.className = 'chain-peer-dot syncing';
    }
  } else {
    if (scannerPill) {
      scannerPill.title = `TRU Blockchain & Ledger (Tip #${finalTip.toLocaleString()})`;
    }
    if (dotScanner && dotScanner.className !== 'chain-peer-dot active') {
      dotScanner.className = 'chain-peer-dot active';
    }
  }
}

function refreshUtxoConfirmations() {
  if (!vaultData || !vaultData.utxos || latestBlockHeight <= 0) return;
  let changed = false;
  for (const u of vaultData.utxos) {
    if (u.block_height > 0) {
      const newConfs = latestBlockHeight >= u.block_height ? (latestBlockHeight - u.block_height + 1) : 1;
      if (u.confirmations !== newConfs) {
        u.confirmations = newConfs;
        changed = true;
      }
    }
  }
  if (changed) {
    renderUtxoTable(vaultData.utxos);
  }
}

let wsReconnectTimer = null;

function initWebSocket() {
  if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
    return;
  }

  if (wsReconnectTimer) {
    clearTimeout(wsReconnectTimer);
    wsReconnectTimer = null;
  }

  if (ws) {
    try {
      ws.onopen = null;
      ws.onmessage = null;
      ws.onclose = null;
      ws.onerror = null;
      ws.close();
    } catch (_) {}
    ws = null;
  }

  const host = window.location.host;
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const wsUrl = `${protocol}//${host}/ws?token=${encodeURIComponent(sessionToken)}`;

  const dot = document.getElementById('dot-ws');
  const text = document.getElementById('text-ws');

  try {
    ws = new WebSocket(wsUrl);

    ws.onopen = () => {
      if (dot) dot.className = "status-dot active";
      if (text) text.innerText = "Online";
      console.log("[WS] Connected to local daemon");
      const tok = sessionToken || window.SERVER_SESSION_TOKEN || "";
      if (tok) {
        ws.send(JSON.stringify({ action: "auth", token: tok }));
      }
      // Initial requests as fallback (daemon already pushes snapshot automatically on connect)
      ws.send(JSON.stringify({ action: "req_vault_status" }));
      ws.send(JSON.stringify({ action: "req_mempool" }));
      ws.send(JSON.stringify({ action: "req_swaps_ledger" }));
      ws.send(JSON.stringify({ action: "req_automation_logs" }));
    };

    ws.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data);
        handleJsonMessage(msg);
      } catch (err) {
        console.warn("[WS] Invalid message received:", event.data, err);
      }
    };

    ws.onclose = () => {
      if (dot) dot.className = "status-dot";
      if (text) text.innerText = "Offline";
      for (const [id, pending] of pendingActionResolvers.entries()) {
        clearTimeout(pending.timer);
        pending.reject(new Error("WebSocket disconnected before action completed"));
      }
      pendingActionResolvers.clear();
      if (!wsReconnectTimer) {
        wsReconnectTimer = setTimeout(initWebSocket, 2500);
      }
    };

    ws.onerror = (err) => {
      console.debug("[WS] Connection status:", err);
    };
  } catch (e) {
    console.warn("[WS] Connection init error:", e);
    if (!wsReconnectTimer) {
      wsReconnectTimer = setTimeout(initWebSocket, 3000);
    }
  }
}

let actionReqCounter = 0;
const pendingActionResolvers = new Map();

function sendSocketAction(action, payload = {}) {
  return new Promise((resolve, reject) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return reject(new Error("Local node WebSocket is disconnected. Reconnecting..."));
    }
    const req_id = ++actionReqCounter;
    const timer = setTimeout(() => {
      if (pendingActionResolvers.has(req_id)) {
        pendingActionResolvers.delete(req_id);
        reject(new Error(`Action '${action}' timed out waiting for daemon response`));
      }
    }, 60000);

    const tok = sessionToken || window.SERVER_SESSION_TOKEN || "";
    pendingActionResolvers.set(req_id, { resolve, reject, timer, action });
    ws.send(JSON.stringify({
      action,
      req_id,
      token: tok,
      payload
    }));
  });
}
window.sendSocketAction = sendSocketAction;

function handleJsonMessage(msg) {
  if (!msg || !msg.type) return;
  const data = msg.data;

  switch (msg.type) {
    case 'init': {
      if (!data) return;
      if (data.block_height > 0) {
        latestBlockHeight = Math.max(latestBlockHeight, Number(data.block_height));
        refreshUtxoConfirmations();
      }
      latestPeerCount = Number(data.p2p_peers) || 0;
      latestVerifiedHeight = Number(data.verified_height) || 0;
      latestTargetHeight = Number(data.target_height) || 0;
      updateHeaderChainDisplay();

      if (data.vault_address) {
        setTextIfChanged('tru-vault-address', data.vault_address);
      }

      const confTru = Number(data.confirmed_atoms) / 1e8;
      const unconfTru = Number(data.unconfirmed_atoms) / 1e8;
      updateTruBalanceDisplay(confTru, unconfTru);
      break;
    }

    case 'vault': {
      if (!data) return;
      if (typeof applyVaultStatus === 'function') {
        applyVaultStatus(data);
      } else {
        const confTru = Number(data.confirmed_tru) || (Number(data.confirmed_atoms) / 1e8) || 0;
        const unconfTru = Number(data.unconfirmed_tru) || (Number(data.unconfirmed_atoms) / 1e8) || 0;
        const totalTru = Number(data.balance_tru) || (confTru + unconfTru);
        const utxos = Array.isArray(data.utxos) ? data.utxos : [];

        vaultData = {
          ...(typeof vaultData !== 'undefined' && vaultData ? vaultData : {}),
          address: data.address || '',
          pubkey: data.pubkey || '',
          balance_atoms: Number(data.balance_atoms) || 0,
          balance_tru: totalTru,
          confirmed_atoms: Number(data.confirmed_atoms) || 0,
          confirmed_tru: confTru,
          unconfirmed_atoms: Number(data.unconfirmed_atoms) || 0,
          unconfirmed_tru: unconfTru,
          base_address: data.base_address || '',
          base_eth_balance: Number(data.base_eth_balance) || 0,
          base_usdc_balance: Number(data.base_usdc_balance) || 0,
          utxo_count: utxos.length,
          mempool_count: Number(data.mempool_count) || 0,
          utxos,
        };

        updateTruBalanceDisplay(confTru, unconfTru);
        if (typeof updateAvailableBalanceLabel === 'function') updateAvailableBalanceLabel();
        setTextIfChanged('modal-avail-bal', `${confTru.toFixed(8)} TRU`);

        const unconfNote = document.getElementById('modal-unconf-note');
        if (unconfNote) {
          if (unconfTru > 0) {
            unconfNote.style.display = 'inline';
            setTextIfChanged(unconfNote, `(+${unconfTru.toFixed(8)})`);
          } else {
            unconfNote.style.display = 'none';
          }
        }

        if (unconfTru > 0) {
          setHtmlIfChanged('header-vault-bal', `${confTru.toFixed(4)} <span style="font-size: 0.72rem; color: var(--tru-gold);">(+${unconfTru.toFixed(2)})</span>`);
        } else {
          setTextIfChanged('header-vault-bal', `${confTru.toFixed(4)} TRU`);
        }

        if (data.address) {
          setTextIfChanged('tru-vault-address', data.address);
          setTextIfChanged('modal-tru-address', data.address);
        }
        setTextIfChanged('btn-vault-utxos', `UTXOs (${utxos.length})`);
        setTextIfChanged('modal-utxo-count', String(utxos.length));
        setTextIfChanged('utxo-tracker-count', `${utxos.length} (${totalTru.toFixed(8)} TRU)`);

        if (typeof renderUtxoTable === 'function') {
          renderUtxoTable(utxos);
        }
      }
      break;
    }

    case 'orderbook_snapshot': {
      if (typeof applyOrderbookSnapshot === 'function') {
        applyOrderbookSnapshot(data);
      }
      break;
    }

    case 'order_added': {
      if (typeof applyOrderAdded === 'function') {
        applyOrderAdded(data);
      }
      break;
    }

    case 'order_removed': {
      if (typeof applyOrderRemoved === 'function') {
        applyOrderRemoved(data.order_id || data);
      }
      break;
    }

    case 'order_matched': {
      if (typeof applyOrderMatched === 'function') {
        applyOrderMatched(data);
      }
      break;
    }

    case 'mempool': {
      const txs = Array.isArray(data) ? data : (data && Array.isArray(data.transactions) ? data.transactions : []);
      const count = txs.length;
      mempoolData = { count, transactions: txs };

      setTextIfChanged('header-mempool-count', `${count} txs`);

      if (typeof renderMempoolRows === 'function' && currentFilter === 'mempool') {
        renderMempoolRows(txs);
      }
      break;
    }

    case 'swaps_ledger': {
      if (!data) return;
      historicalLedgerData = data;
      latestVerifiedHeight = Number(data.verified_height) || latestVerifiedHeight;
      latestTargetHeight = Number(data.target_height) || latestTargetHeight;
      latestSwapCount = Number(data.total_swaps) || (Array.isArray(data.swaps) ? data.swaps.length : 0);
      updateHeaderChainDisplay();

      if (currentFilter === 'ledger' && typeof renderLedgerRows === 'function') {
        renderLedgerRows(data.swaps || []);
      }
      if (typeof renderActiveSwapsTracker === 'function') {
        renderActiveSwapsTracker();
      }
      if (typeof updateAuditModalTimeline === 'function') {
        updateAuditModalTimeline();
      }
      break;
    }

    case 'automation_log': {
      if (typeof handleAutomationLog === 'function') {
        handleAutomationLog(data);
      }
      break;
    }

    case 'automation_logs': {
      if (typeof handleAutomationLogsList === 'function') {
        handleAutomationLogsList(data);
      }
      break;
    }

    case 'swap_progress': {
      if (!data) return;
      const step = Number(data.step) || 1;
      const status = Number(data.status) || 0;
      for (let s = 1; s <= 4; s++) {
        const el = document.getElementById(`step-${s}`);
        if (!el) continue;
        if (s < step || (s === step && status === 1)) {
          el.className = "step-item completed";
        } else if (s === step && status === 0) {
          el.className = "step-item active";
        } else {
          el.className = "step-item";
        }
      }
      break;
    }

    case 'withdraw_result': {
      const btn = document.getElementById('btn-modal-do-withdraw');
      if (btn) {
        btn.disabled = false;
        btn.innerText = "Broadcast Withdrawal";
      }
      const statusBox = document.getElementById('modal-withdraw-status');
      if (data && data.success) {
        if (statusBox) {
          statusBox.style.display = 'block';
          statusBox.style.background = 'rgba(0, 230, 118, 0.12)';
          statusBox.style.color = 'var(--accent-green)';
          statusBox.style.border = '1px solid rgba(0, 230, 118, 0.4)';
          statusBox.innerHTML = `
            <strong>✓ Withdrawal Broadcasted</strong><br>
            <span style="font-family: var(--font-mono); font-size: 0.75rem; word-break: break-all;">TXID: ${data.txid}</span>
          `;
          setTimeout(() => {
            try { statusBox.scrollIntoView({ behavior: 'smooth', block: 'nearest' }); } catch (e) {}
          }, 50);
        }
        if (typeof showToast === 'function') showToast(`✓ Withdrawal broadcasted: ${(data.txid || '').slice(0, 12)}...`);
        const destInput = document.getElementById('modal-withdraw-dest');
        const amtInput = document.getElementById('modal-withdraw-amount');
        if (destInput) destInput.value = "";
        if (amtInput) amtInput.value = "";
        if (typeof fetchVaultStatus === 'function') fetchVaultStatus(true);
      } else {
        if (statusBox) {
          statusBox.style.display = 'block';
          statusBox.style.background = 'rgba(255, 51, 102, 0.12)';
          statusBox.style.color = 'var(--accent-red)';
          statusBox.style.border = '1px solid rgba(255, 51, 102, 0.4)';
          statusBox.innerHTML = `❌ Error: ${(data && data.error) || "Withdrawal failed"}`;
        }
      }
      break;
    }

    case 'vault_export': {
      if (typeof handleExportSocketResponse === 'function') {
        if (data && data.success) {
          exportKeysData = {
            address: data.address,
            priv_key_hex: data.priv_key_hex,
            priv_key_wif: data.priv_key_wif,
            mnemonic: data.mnemonic,
            base_address: data.base_address,
            base_priv_key_hex: data.base_priv_key_hex,
            base_mnemonic: data.base_mnemonic
          };
          handleExportSocketResponse(exportKeysData, null);
        } else {
          handleExportSocketResponse(null, (data && data.error) || "Export failed");
        }
      }
      break;
    }

    case 'qr_frame': {
      if (!data || !data.img) return;
      const streamImg = document.getElementById('qr-camera-stream');
      if (streamImg) {
        streamImg.src = data.img;
        streamImg.style.display = 'block';
      }
      const video = document.getElementById('qr-video');
      if (video) video.style.display = 'none';
      const loadingEl = document.getElementById('qr-loading-overlay');
      if (loadingEl) loadingEl.style.display = 'none';
      if (typeof _qrState !== 'undefined') {
        _qrState.active = true;
      }
      break;
    }

    case 'qr_scanned': {
      if (!data || !data.code) return;
      if (typeof onQrSuccess === 'function') {
        onQrSuccess(data.code);
      }
      break;
    }

    case 'action_result': {
      if (!data) return;
      const reqId = Number(data.req_id);
      const pending = pendingActionResolvers.get(reqId);
      if (pending) {
        clearTimeout(pending.timer);
        pendingActionResolvers.delete(reqId);
        if (data.success) {
          pending.resolve(data.data !== undefined ? data.data : data);
        } else {
          pending.reject(new Error(data.error || `Action ${pending.action} failed`));
        }
      }
      break;
    }

    case 'pong':
      break;

    default:
      console.debug("[WS] Unhandled message type:", msg.type, data);
  }
}

function requestVaultExportOverSocket(token) {
  if (!ws || ws.readyState !== WebSocket.OPEN) {
    console.warn("[SOCKET] Local node WebSocket is not connected for export");
    return false;
  }
  ws.send(JSON.stringify({
    action: "req_vault_export",
    token: token || sessionToken || ""
  }));
  return true;
}
window.requestVaultExportOverSocket = requestVaultExportOverSocket;

// Export function for initial socket connection if DOM is already ready
window.initWebSocket = initWebSocket;
