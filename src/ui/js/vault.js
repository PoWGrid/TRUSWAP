// --- TRU Vault & UTXO Management ---
let vaultExportData = null;

function applyVaultStatus(data) {
  if (!data) return;
  vaultData = {
    ...(typeof vaultData !== 'undefined' && vaultData ? vaultData : {}),
    ...data
  };

  const unconfirmedTru = data.unconfirmed_tru !== undefined ? data.unconfirmed_tru : ((data.unconfirmed_atoms || 0) / 1e8);
  const confirmedTru = data.confirmed_tru !== undefined ? data.confirmed_tru : (data.confirmed_atoms !== undefined ? data.confirmed_atoms / 1e8 : (data.balance_tru || 0));
  const totalTru = data.balance_tru !== undefined ? data.balance_tru : (confirmedTru + unconfirmedTru);
  const truFormatted = totalTru.toFixed(8);
  const utxos = Array.isArray(data.utxos) ? data.utxos : [];

  updateTruBalanceDisplay(confirmedTru, unconfirmedTru);
  if (typeof updateAvailableBalanceLabel === 'function') updateAvailableBalanceLabel();
  setTextIfChanged('modal-avail-bal', `${confirmedTru.toFixed(8)} TRU`);
  const unconfNote = document.getElementById('modal-unconf-note');
  if (unconfNote) {
    if (unconfirmedTru > 0) {
      unconfNote.style.display = 'inline';
      setTextIfChanged(unconfNote, `(+${unconfirmedTru.toFixed(8)})`);
    } else {
      unconfNote.style.display = 'none';
    }
  }
  if (unconfirmedTru > 0) {
    setHtmlIfChanged('header-vault-bal', `${confirmedTru.toFixed(4)} <span style="font-size: 0.72rem; color: var(--tru-gold);">(+${unconfirmedTru.toFixed(2)})</span>`);
  } else {
    setTextIfChanged('header-vault-bal', `${confirmedTru.toFixed(4)} TRU`);
  }

  setTextIfChanged('tru-vault-address', data.address || "");
  setTextIfChanged('modal-tru-address', data.address || "");
  setTextIfChanged('btn-vault-utxos', `UTXOs (${utxos.length})`);
  setTextIfChanged('modal-utxo-count', String(utxos.length));
  setTextIfChanged('utxo-tracker-count', `${utxos.length} (${truFormatted} TRU)`);

  renderUtxoTable(utxos);

  if (data.chain_id) BASE_CHAIN_ID = data.chain_id;
  if (data.explorer_url) BASE_EXPLORER_URL = data.explorer_url;
  if (data.network_name) BASE_NETWORK_NAME = data.network_name;
  if (typeof data.is_testnet === 'boolean') IS_TESTNET = data.is_testnet;
  if (data.base_htlc_contract) BASE_HTLC_ADDR = data.base_htlc_contract;
  if (data.base_usdc_contract) BASE_USDC_ADDR = data.base_usdc_contract;
  if (data.relay_node_url) RELAY_NODE_URL = data.relay_node_url;

  // Update dynamic network badges and labels if present
  const netBadge = document.getElementById('base-network-info');
  if (netBadge) setTextIfChanged(netBadge, 'Base (8453)');
  const depNet = document.getElementById('base-deposit-net-label');
  if (depNet) setTextIfChanged(depNet, `${BASE_NETWORK_NAME} EVM`);
  const depUsdc = document.getElementById('base-deposit-usdc-label');
  if (depUsdc) setTextIfChanged(depUsdc, `${BASE_NETWORK_NAME} USDC`);
  const depEth = document.getElementById('base-deposit-eth-label');
  if (depEth) setTextIfChanged(depEth, 'Base ETH');
  const footNet = document.getElementById('base-footer-net-name');
  if (footNet) setTextIfChanged(footNet, BASE_NETWORK_NAME);

  const htlcContractEl = document.getElementById('explorer-htlc-address');
  if (htlcContractEl) setTextIfChanged(htlcContractEl, BASE_HTLC_ADDR);
  const htlcLink = document.getElementById('link-basescan-htlc');
  if (htlcLink) htlcLink.href = `${BASE_EXPLORER_URL}/address/${BASE_HTLC_ADDR}`;

  const usdcContractEl = document.getElementById('explorer-usdc-address');
  if (usdcContractEl) setTextIfChanged(usdcContractEl, BASE_USDC_ADDR);
  const usdcLink = document.getElementById('link-basescan-usdc');
  if (usdcLink) usdcLink.href = `${BASE_EXPLORER_URL}/token/${BASE_USDC_ADDR}`;

  // Base Vault UI elements
  if (data.base_address) {
    setTextIfChanged('base-wallet-address', data.base_address);
    setTextIfChanged('modal-base-address', data.base_address);
    setTextIfChanged('explorer-base-address', data.base_address);
    setTextIfChanged('base-export-addr-val', data.base_address);
    const linkBaseScan = document.getElementById('link-basescan-addr');
    if (linkBaseScan) linkBaseScan.href = `${BASE_EXPLORER_URL}/address/${data.base_address}`;
    const linkCardExplorer = document.getElementById('link-base-card-explorer');
    if (linkCardExplorer) linkCardExplorer.href = `${BASE_EXPLORER_URL}/address/${data.base_address}`;
  }
  const usdcBal = data.base_usdc_balance !== undefined ? data.base_usdc_balance : 0;
  const ethBal = data.base_eth_balance !== undefined ? data.base_eth_balance : 0;
  setTextIfChanged('base-usdc-balance', floorDecimals(usdcBal, 2).toFixed(2));
  setTextIfChanged('modal-base-usdc-avail', (usdcBal > 0 ? (usdcBal % 1 === 0 ? usdcBal.toFixed(2) : floorDecimals(usdcBal, 4).toString()) : "0.00"));
  setTextIfChanged('base-eth-balance', `${floorDecimals(ethBal, 6).toFixed(6)} ETH`);
  setTextIfChanged('modal-base-eth-avail', floorDecimals(ethBal, 6).toFixed(6));
  const activeAsset = typeof baseWithdrawAsset !== 'undefined' ? baseWithdrawAsset : 'USDC';
  setTextIfChanged('base-modal-avail-bal', activeAsset === 'USDC' 
    ? `${usdcBal > 0 ? floorDecimals(usdcBal, 6).toString() : "0.00"} USDC` 
    : `${floorDecimals(ethBal, 6).toFixed(6)} ETH`);

  const modal = document.getElementById('vault-modal');
  if (modal && modal.classList.contains('open')) {
    const depPane = document.getElementById('vtab-pane-deposit');
    if (depPane && depPane.classList.contains('active')) {
      renderVaultQrCode();
    }
  }

  const baseModal = document.getElementById('base-vault-modal');
  if (baseModal && baseModal.classList.contains('open')) {
    const bDepPane = document.getElementById('btab-pane-deposit');
    if (bDepPane && bDepPane.classList.contains('active')) {
      renderBaseQrCode();
    }
    const bExpPane = document.getElementById('btab-pane-export');
    if (bExpPane && bExpPane.classList.contains('active')) {
      renderBaseExport();
    }
  }

  if (typeof updateTradeButtonText === 'function') {
    updateTradeButtonText();
  }
}
window.applyVaultStatus = applyVaultStatus;

function fetchVaultStatus(force = false) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ action: "req_vault_status" }));
    ws.send(JSON.stringify({ action: "req_mempool" }));
    ws.send(JSON.stringify({ action: "req_swaps_ledger" }));
  }
}
window.fetchVaultStatus = fetchVaultStatus;

let lastUtxosSig = "";
function renderUtxoTable(utxos) {
  const tbody = document.getElementById('utxo-table-body');
  if (!tbody) return;

  const sig = JSON.stringify((utxos || []).map(u => [u.txid, u.vout, u.amount_tru, u.block_height, u.confirmations]));
  if (sig === lastUtxosSig) return;
  lastUtxosSig = sig;

  if (!utxos || utxos.length === 0) {
    tbody.innerHTML = `<tr><td colspan="4" style="text-align: center; color: var(--text-dim); padding: 24px;">No unspent outputs in local vault yet. Deposit TRU to fund atomic swaps.</td></tr>`;
    return;
  }

  tbody.innerHTML = utxos.map(u => {
    const shortTxid = u.txid.length > 16 ? `${u.txid.slice(0, 7)}...${u.txid.slice(-5)}` : u.txid;
    const outpoint = `${shortTxid}:${u.vout}`;
    const isMempool = !u.block_height || u.block_height === 0 || u.confirmations === 0 || u.is_mempool;
    const blockDisplay = isMempool 
      ? `<span style="color: var(--tru-gold); font-size: 0.75rem;">⚡ Mempool</span>`
      : `#${u.block_height}`;
    const confBadge = isMempool
      ? `<span class="badge-confs" style="color: #ffb800; background: rgba(255,184,0,0.1); border-color: rgba(255,184,0,0.4); font-weight: 700;">⚡ Mempool</span>`
      : `<span class="badge-confs">✓ ${u.confirmations} confs</span>`;

    return `
      <tr>
        <td style="font-family: var(--font-mono); font-weight: 700; color: #38bdf8; text-align: center; cursor: pointer;" title="Click to view transaction in TRU Explorer" onclick="if(typeof openTruTxModal==='function') openTruTxModal('${u.txid}', event)">${outpoint} 🔍</td>
        <td style="font-family: var(--font-mono); font-weight: 700; color: #00f0ff; text-align: center;">${blockDisplay}</td>
        <td style="font-family: var(--font-mono); font-weight: 700; color: #ffb800; text-align: center;">${u.amount_tru.toFixed(4)} TRU</td>
        <td style="text-align: center;">${confBadge}</td>
      </tr>
    `;
  }).join('');
}

function openVaultModal(tab = 'deposit') {
  document.body.classList.add('modal-open');
  document.getElementById('vault-modal').classList.add('open');
  switchVaultTab(tab);
  fetchVaultStatus(true);
}

function closeVaultModal() {
  document.body.classList.remove('modal-open');
  document.getElementById('vault-modal').classList.remove('open');
  const statusBox = document.getElementById('modal-withdraw-status');
  if (statusBox) statusBox.style.display = 'none';
  if (currentFilter === 'mempool') {
    renderMempoolRows(mempoolData.transactions);
  } else {
    renderOrderbook();
  }
  exportRevealed.privhex = false;
  exportRevealed.privwif = false;
  exportRevealed.mnemonic = false;
  exportRevealed.base_priv = false;
  if (vaultExportData) renderVaultExport();
}

function switchVaultTab(tab) {
  const tabs = ['deposit', 'withdraw', 'utxos', 'export'];
  tabs.forEach(t => {
    const btn = document.getElementById(`vtab-btn-${t}`);
    const pane = document.getElementById(`vtab-pane-${t}`);
    if (btn) btn.classList.toggle('active', t === tab);
    if (pane) pane.classList.toggle('active', t === tab);
  });

  if (tab === 'deposit') {
    setTimeout(() => renderVaultQrCode(true), 50);
  } else if (tab === 'withdraw') {
    validateWithdrawInput();
  } else if (tab === 'export') {
    fetchVaultExport();
  }
}

function handleExportSocketResponse(data, err) {
  const container = document.getElementById('export-content-container');
  const errorBox = document.getElementById('export-error-box');
  const loadingBox = document.getElementById('export-loading-box');

  const baseContainer = document.getElementById('base-export-content-container');
  const baseErrorBox = document.getElementById('base-export-error-box');
  const baseLoadingBox = document.getElementById('base-export-loading-box');

  if (loadingBox) loadingBox.style.display = 'none';
  if (baseLoadingBox) baseLoadingBox.style.display = 'none';

  if (data) {
    vaultExportData = data;
    if (container) container.style.display = 'block';
    if (errorBox) errorBox.style.display = 'none';
    if (baseContainer) baseContainer.style.display = 'block';
    if (baseErrorBox) baseErrorBox.style.display = 'none';
    renderVaultExport();
    renderBaseExport();
  } else {
    if (container) container.style.display = 'none';
    if (errorBox) {
      errorBox.style.display = 'block';
      errorBox.innerText = `Export error: ${err || "Failed to retrieve credentials"}`;
    }
    if (baseContainer) baseContainer.style.display = 'none';
    if (baseErrorBox) {
      baseErrorBox.style.display = 'block';
      baseErrorBox.innerText = `Base export error: ${err || "Failed to retrieve credentials"}`;
    }
  }
}
window.handleExportSocketResponse = handleExportSocketResponse;

// --- Key Export & Seed Phrase ---
async function fetchVaultExport() {
  const container = document.getElementById('export-content-container');
  const errorBox = document.getElementById('export-error-box');
  const loadingBox = document.getElementById('export-loading-box');

  if (vaultExportData) {
    if (loadingBox) loadingBox.style.display = 'none';
    if (container) container.style.display = 'block';
    if (errorBox) errorBox.style.display = 'none';
    renderVaultExport();
    return;
  }

  if (loadingBox) loadingBox.style.display = 'block';
  if (container) container.style.display = 'none';
  if (errorBox) errorBox.style.display = 'none';

  if (typeof requestVaultExportOverSocket === 'function' && ws && ws.readyState === WebSocket.OPEN) {
    const tok = sessionToken || window.SERVER_SESSION_TOKEN || "";
    if (requestVaultExportOverSocket(tok)) {
      return;
    }
  }

  if (loadingBox) loadingBox.style.display = 'none';
  if (container) container.style.display = 'none';
  if (errorBox) {
    errorBox.style.display = 'block';
    errorBox.innerText = "Export error: WebSocket connection is not open";
  }
}

function renderVaultExport() {
  if (!vaultExportData) return;
  const addrEl = document.getElementById('export-addr-val');
  if (addrEl) addrEl.innerText = vaultExportData.address;
  const baseAddrEl = document.getElementById('export-base-addr-val');
  if (baseAddrEl) baseAddrEl.innerText = vaultExportData.base_address || 'N/A';

  updateExportFieldVisibility('privhex');
  updateExportFieldVisibility('privwif');
  updateExportFieldVisibility('mnemonic');
  updateExportFieldVisibility('base_priv');
}

function toggleExportReveal(field) {
  exportRevealed[field] = !exportRevealed[field];
  updateExportFieldVisibility(field);
}

function updateExportFieldVisibility(field) {
  if (!vaultExportData) return;
  const isRevealed = exportRevealed[field];
  const valEl = document.getElementById(`export-${field}-val`);
  const btn = document.getElementById(`btn-toggle-${field}`);

  if (valEl) {
    if (isRevealed) {
      if (field === 'privhex') valEl.innerText = vaultExportData.priv_key_hex;
      else if (field === 'privwif') valEl.innerText = vaultExportData.priv_key_wif;
      else if (field === 'mnemonic') valEl.innerText = vaultExportData.mnemonic;
      else if (field === 'base_priv') valEl.innerText = vaultExportData.base_priv_key_hex;
      valEl.classList.remove('export-blur');
    } else {
      let placeholder = '';
      if (field === 'privhex') {
        placeholder = '••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••';
      } else if (field === 'privwif') {
        placeholder = '••••••••••••••••••••••••••••••••••••••••••••••••••••';
      } else if (field === 'mnemonic') {
        placeholder = '•••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• ••••';
      } else if (field === 'base_priv') {
        placeholder = '••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••';
      }
      valEl.innerText = placeholder;
      valEl.classList.add('export-blur');
    }
  }
  if (btn) {
    if (field === 'mnemonic') {
      btn.innerText = isRevealed ? '🔒 Hide Words' : '👁 Show Words';
    } else {
      btn.innerText = isRevealed ? '🔒 Hide' : '👁 Show';
    }
  }
}

function copyExportField(field, msg) {
  if (!vaultExportData) return;
  let text = '';
  if (field === 'address') text = vaultExportData.address;
  else if (field === 'privhex') text = vaultExportData.priv_key_hex;
  else if (field === 'privwif') text = vaultExportData.priv_key_wif;
  else if (field === 'mnemonic') text = vaultExportData.mnemonic;
  else if (field === 'base_address') text = vaultExportData.base_address;
  else if (field === 'base_priv') text = vaultExportData.base_priv_key_hex;

  if (text) {
    copyToClipboard(text, msg);
  }
}

// --- QR Code Rendering ---
function renderVaultQrCode(force = false) {
  const addr = (vaultData && vaultData.address) ? vaultData.address : (document.getElementById('tru-vault-address') ? document.getElementById('tru-vault-address').innerText : '');
  if (!addr || addr.includes('Detecting') || addr.includes('Loading')) return;
  const container = document.getElementById('vault-qrcode');
  if (!container) return;

  if (!force && qrRenderedForAddr === addr && container.querySelector('canvas')) {
    const cv = container.querySelector('canvas');
    if (cv) cv.style.setProperty('display', 'block', 'important');
    return;
  }

  container.innerHTML = "";

  if (window.QRCode) {
    try {
      new QRCode(container, {
        text: addr,
        width: 240,
        height: 240,
        colorDark: "#07090e",
        colorLight: "#ffffff",
        correctLevel: QRCode.CorrectLevel.M
      });
      qrRenderedForAddr = addr;
      const cv = container.querySelector('canvas');
      if (cv) cv.style.setProperty('display', 'block', 'important');
      setTimeout(() => {
        const cv = container.querySelector('canvas');
        if (cv) cv.style.setProperty('display', 'block', 'important');
      }, 30);
    } catch (e) {
      console.warn("TRU QRCode error:", e);
    }
  }
}

// --- Fee Calculation & Withdrawal Validation ---
function calculateExactFee(amountTru) {
  const amountAtoms = BigInt(Math.floor((parseFloat(amountTru) || 0) * 1e8));
  const confirmedUtxos = (vaultData.utxos || []).filter(u => u.confirmations > 0 && u.block_height > 0);
  const confAtoms = BigInt(vaultData.confirmed_atoms !== undefined ? vaultData.confirmed_atoms : (vaultData.balance_atoms || 0));

  const sweepVin = Math.max(1, confirmedUtxos.length);
  const varintSweepVin = sweepVin < 0xfd ? 1 : 3;
  const sweepBytes = 8 + varintSweepVin + (sweepVin * 148) + 1 + 34; // 1 output
  const sweepFeeAtoms = BigInt(sweepBytes * 1 + 10);

  const isSweep = confAtoms > 0n && (amountAtoms === 0n || amountAtoms >= (confAtoms - sweepFeeAtoms));

  let vinCount = 1;
  if (confirmedUtxos.length > 0) {
    if (isSweep) {
      vinCount = sweepVin;
    } else {
      let sum = 0n;
      let count = 0;
      for (const u of confirmedUtxos) {
        sum += BigInt(u.amount_atoms || 0);
        count++;
        if (sum >= amountAtoms) break;
      }
      vinCount = Math.max(1, count);
    }
  }

  const voutCount = isSweep && confAtoms > 0n ? 1 : 2;
  const varintVin = vinCount < 0xfd ? 1 : 3;
  const varintVout = voutCount < 0xfd ? 1 : 3;
  const txBytes = 8 + varintVin + (vinCount * 148) + varintVout + (voutCount * 34);

  const feeAtoms = BigInt(txBytes * 1 + 10);
  const feeTru = Number(feeAtoms) / 1e8;

  return {
    vinCount,
    voutCount,
    txBytes,
    feeAtoms,
    feeTru,
    isSweep
  };
}

function validateWithdrawInput() {
  const destInput = document.getElementById('modal-withdraw-dest');
  const amtInput = document.getElementById('modal-withdraw-amount');
  const hint = document.getElementById('modal-dest-hint');
  const netDisplay = document.getElementById('modal-withdraw-net');
  const btn = document.getElementById('btn-modal-do-withdraw');
  const feeText = document.getElementById('modal-fee-atoms-text');
  const statusBox = document.getElementById('modal-withdraw-status');

  const dest = destInput ? destInput.value.trim() : "";
  const amt = amtInput ? parseFloat(amtInput.value) || 0 : 0;
  const amtAtoms = BigInt(Math.floor(amt * 1e8));

  const calc = calculateExactFee(amt);
  const feeAtoms = calc.feeAtoms;
  const feeTru = calc.feeTru;

  if (feeText) {
    feeText.innerText = `${feeTru.toFixed(8)} TRU`;
  }

  let validDest = false;
  if (dest.startsWith('T') && dest.length >= 33 && dest.length <= 35) {
    validDest = true;
    if (hint) {
      hint.innerText = "✓ Valid TRU Mainnet Address format";
      hint.style.color = "var(--accent-green)";
    }
  } else if (dest.length > 0) {
    if (hint) {
      hint.innerText = "Invalid TRU address (must start with 'T', 33-35 chars)";
      hint.style.color = "var(--accent-red)";
    }
  } else {
    if (hint) {
      hint.innerText = "Must start with 'T' (Mainnet Base58Check)";
      hint.style.color = "var(--text-dim)";
    }
  }

  if (netDisplay) {
    const totalDeducted = amt > 0 ? (amt + feeTru) : 0;
    netDisplay.innerText = `${totalDeducted.toFixed(8)} TRU`;
  }

  const confAtoms = BigInt(vaultData.confirmed_atoms !== undefined ? vaultData.confirmed_atoms : (vaultData.balance_atoms || 0));
  const maxAtoms = confAtoms > feeAtoms ? confAtoms - feeAtoms : 0n;
  const maxTru = Number(maxAtoms) / 1e8;

  if (amt > maxTru && maxTru > 0) {
    amtInput.value = maxTru.toFixed(8);
    amt = maxTru;
    amtAtoms = maxAtoms;
    if (netDisplay) {
      netDisplay.innerText = `${(amt + feeTru).toFixed(8)} TRU`;
    }
  }

  const totalNeeded = amtAtoms + feeAtoms;
  const hasEnoughConfirmed = amt > 0 && totalNeeded <= confAtoms;
  const canSubmit = validDest && hasEnoughConfirmed;
  if (btn) {
    btn.disabled = !canSubmit;
  }

  if (amtAtoms > 0n && totalNeeded > confAtoms) {
    if (statusBox) {
      statusBox.style.display = 'block';
      statusBox.style.background = 'rgba(255, 184, 0, 0.08)';
      statusBox.style.color = 'var(--tru-gold)';
      statusBox.style.border = '1px solid rgba(255, 184, 0, 0.3)';
      const unconfAtoms = BigInt(vaultData.unconfirmed_atoms || 0);
      if (unconfAtoms > 0n) {
        const unconfTru = (Number(unconfAtoms) / 1e8).toFixed(4);
        statusBox.innerHTML = `⚠️ Cannot spend unconfirmed change (${unconfTru} TRU) from mempool. Please wait for 1 block confirmation (~15-30s). Spendable: ${(Number(confAtoms) / 1e8).toFixed(8)} TRU.`;
      } else {
        statusBox.innerHTML = `⚠️ Insufficient confirmed balance: ${(Number(confAtoms) / 1e8).toFixed(8)} TRU available.`;
      }
    }
  } else if (statusBox && statusBox.innerText.startsWith('⚠️')) {
    statusBox.style.display = 'none';
  }
}

function setWithdrawPercent(pct) {
  if (pct >= 1.0) {
    setMaxWithdraw();
    return;
  }
  const confAtoms = BigInt(vaultData.confirmed_atoms !== undefined ? vaultData.confirmed_atoms : (vaultData.balance_atoms || 0));
  if (confAtoms === 0n) return;
  const targetAtoms = Math.floor(Number(confAtoms) * pct);
  const amtInput = document.getElementById('modal-withdraw-amount');
  if (amtInput) {
    amtInput.value = (targetAtoms / 1e8).toFixed(8);
    validateWithdrawInput();
  }
}

function setMaxWithdraw() {
  const confAtoms = BigInt(vaultData.confirmed_atoms !== undefined ? vaultData.confirmed_atoms : (vaultData.balance_atoms || 0));
  if (confAtoms === 0n) {
    const amtInput = document.getElementById('modal-withdraw-amount');
    if (amtInput) amtInput.value = "0.00000000";
    validateWithdrawInput();
    return;
  }

  const confirmedUtxos = (vaultData.utxos || []).filter(u => u.confirmations > 0 && u.block_height > 0);
  const vinCount = Math.max(1, confirmedUtxos.length);
  const voutCount = 1;
  const varintVin = vinCount < 0xfd ? 1 : 3;
  const varintVout = 1;
  const txBytes = 8 + varintVin + (vinCount * 148) + varintVout + (voutCount * 34);
  const feeAtoms = BigInt(txBytes * 1 + 10);

  const maxAtoms = confAtoms > feeAtoms ? confAtoms - feeAtoms : 0n;
  const amtInput = document.getElementById('modal-withdraw-amount');
  if (amtInput) {
    amtInput.value = (Number(maxAtoms) / 1e8).toFixed(8);
    validateWithdrawInput();
  }
}

async function submitVaultWithdraw() {
  const dest = document.getElementById('modal-withdraw-dest').value.trim();
  const amt = parseFloat(document.getElementById('modal-withdraw-amount').value) || 0;
  const statusBox = document.getElementById('modal-withdraw-status');
  const btn = document.getElementById('btn-modal-do-withdraw');

  if (!dest || amt <= 0) {
    if (typeof showToast === 'function') showToast("⚠️ Please enter a valid destination address and amount > 0 TRU", "warning");
    if (statusBox) {
      statusBox.style.display = 'block';
      statusBox.style.background = 'rgba(239, 68, 68, 0.12)';
      statusBox.style.color = '#f87171';
      statusBox.style.border = '1px solid rgba(239, 68, 68, 0.3)';
      statusBox.innerText = "⚠️ Please enter a valid destination address and amount greater than 0 TRU";
    }
    return;
  }

  const confAtoms = BigInt(vaultData.confirmed_atoms !== undefined ? vaultData.confirmed_atoms : (vaultData.balance_atoms || 0));
  const amtAtoms = BigInt(Math.floor(amt * 1e8));
  const calc = calculateExactFee(amt);
  if (amtAtoms + calc.feeAtoms > confAtoms) {
    const unconfAtoms = BigInt(vaultData.unconfirmed_atoms || 0);
    const errMsg = unconfAtoms > 0n
      ? "⚠️ Cannot spend unconfirmed coins from mempool. Please wait for block confirmation (~15-30s)."
      : "⚠️ Insufficient confirmed balance for this withdrawal amount + network fee.";
    if (typeof showToast === 'function') showToast(errMsg, "warning");
    if (statusBox) {
      statusBox.style.display = 'block';
      statusBox.style.background = 'rgba(239, 68, 68, 0.12)';
      statusBox.style.color = '#f87171';
      statusBox.style.border = '1px solid rgba(239, 68, 68, 0.3)';
      statusBox.innerText = errMsg;
    }
    return;
  }

  btn.disabled = true;
  btn.innerText = "Broadcasting withdrawal...";
  statusBox.style.display = 'block';
  statusBox.style.background = 'rgba(0, 240, 255, 0.08)';
  statusBox.style.color = 'var(--accent-cyan)';
  statusBox.style.border = '1px solid rgba(0, 240, 255, 0.3)';
  statusBox.innerHTML = `⏳ Signing & relaying transaction...`;
  setTimeout(() => {
    try { statusBox.scrollIntoView({ behavior: 'smooth', block: 'nearest' }); } catch (e) {}
  }, 50);

  if (!ws || ws.readyState !== WebSocket.OPEN) {
    if (typeof showToast === 'function') showToast("⚠️ WebSocket is not connected.", "error");
    if (statusBox) {
      statusBox.style.background = 'rgba(239, 68, 68, 0.12)';
      statusBox.style.color = '#f87171';
      statusBox.style.border = '1px solid rgba(239, 68, 68, 0.3)';
      statusBox.innerText = "⚠️ WebSocket is not connected.";
    }
    btn.disabled = false;
    btn.innerText = "Broadcast Withdrawal";
    return;
  }

  const tok = sessionToken || window.SERVER_SESSION_TOKEN || "";
  ws.send(JSON.stringify({
    action: "req_withdraw",
    token: tok,
    destination: dest,
    amount_tru: amt
  }));
}

function copyTruAddress() {
  const addr = (vaultData && vaultData.address) ? vaultData.address : document.getElementById('tru-vault-address').innerText;
  if (addr && !addr.includes('Detecting')) {
    copyToClipboard(addr.trim(), "✓ TRU Address copied to clipboard!");
  }
}

function copyBaseAddress() {
  const addr = (vaultData && vaultData.base_address) ? vaultData.base_address : (document.getElementById('base-wallet-address') ? document.getElementById('base-wallet-address').innerText : '');
  if (addr && !addr.includes('Detecting')) {
    copyToClipboard(addr.trim(), "✓ Base Address copied!");
  }
}

let baseWithdrawAsset = 'USDC';
let baseQrRenderedForAddr = '';
let baseExportRevealed = { priv: false, mnemonic: false };
let currentBaseGasEstimate = null;
let isFetchingBaseGas = false;

async function fetchBaseGasEstimate() {
  if (isFetchingBaseGas) return;
  isFetchingBaseGas = true;
  const gasSpan = document.getElementById('base-withdraw-gas-fee');
  try {
    if (typeof sendSocketAction === 'function') {
      const data = await sendSocketAction('get_base_gas_estimate', { asset: baseWithdrawAsset });
      if (data && (data.status === 'ok' || data.total_fee_eth !== undefined)) {
        currentBaseGasEstimate = data;
        if (gasSpan) {
          gasSpan.innerText = data.formatted_fee || `~${Number(data.total_fee_eth).toFixed(6)} ETH`;
          gasSpan.title = `L2 Gas Price: ${(data.gas_price_gwei || 0).toFixed(3)} Gwei | Gas Limit: ${data.gas_limit || 0} | L1 Fee: ~${(data.l1_fee_eth || 0).toFixed(8)} ETH`;
        }
      }
    }
  } catch (e) {
    console.warn("Failed to fetch Base gas estimate over socket:", e);
    if (gasSpan && !currentBaseGasEstimate) {
      gasSpan.innerText = "~0.000008 ETH (~0.007 Gwei)";
    }
  } finally {
    isFetchingBaseGas = false;
    validateBaseWithdrawInput();
  }
}

function openBaseModal(tab = 'deposit') {
  document.body.classList.add('modal-open');
  const modal = document.getElementById('base-vault-modal');
  if (modal) modal.classList.add('open');
  switchBaseTab(tab);
  fetchVaultStatus(true);
}

function closeBaseModal() {
  document.body.classList.remove('modal-open');
  const modal = document.getElementById('base-vault-modal');
  if (modal) modal.classList.remove('open');
  const statusBox = document.getElementById('base-withdraw-status');
  if (statusBox) statusBox.style.display = 'none';
  baseExportRevealed.priv = false;
  baseExportRevealed.mnemonic = false;
  if (vaultExportData) renderBaseExport();
}

function switchBaseTab(tab) {
  const tabs = ['deposit', 'withdraw', 'explorer', 'export'];
  tabs.forEach(t => {
    const btn = document.getElementById(`btab-btn-${t}`);
    const pane = document.getElementById(`btab-pane-${t}`);
    if (btn) btn.classList.toggle('active', t === tab);
    if (pane) pane.classList.toggle('active', t === tab);
  });

  if (tab === 'deposit') {
    setTimeout(() => renderBaseQrCode(true), 50);
  } else if (tab === 'withdraw') {
    fetchBaseGasEstimate();
    validateBaseWithdrawInput();
  } else if (tab === 'explorer') {
    const addr = (vaultData && vaultData.base_address) ? vaultData.base_address : (document.getElementById('base-wallet-address') ? document.getElementById('base-wallet-address').innerText : '');
    setTextIfChanged('explorer-base-address', addr || 'Loading...');
    const link = document.getElementById('link-basescan-addr');
    if (link && addr && !addr.includes('Detecting')) {
      link.href = `${BASE_EXPLORER_URL}/address/${addr}`;
    }
  } else if (tab === 'export') {
    fetchBaseExport();
  }
}

function renderBaseQrCode(force = false) {
  const addr = (vaultData && vaultData.base_address) ? vaultData.base_address : (document.getElementById('base-wallet-address') ? document.getElementById('base-wallet-address').innerText : '');
  if (!addr || addr.includes('Detecting') || addr.includes('Loading')) return;
  const container = document.getElementById('base-vault-qrcode');
  if (!container) return;

  if (!force && baseQrRenderedForAddr === addr && container.querySelector('canvas')) {
    const cv = container.querySelector('canvas');
    if (cv) cv.style.setProperty('display', 'block', 'important');
    return;
  }

  container.innerHTML = "";

  if (window.QRCode) {
    try {
      new QRCode(container, {
        text: addr,
        width: 240,
        height: 240,
        colorDark: "#07090e",
        colorLight: "#ffffff",
        correctLevel: QRCode.CorrectLevel.M
      });
      baseQrRenderedForAddr = addr;
      const cv = container.querySelector('canvas');
      if (cv) cv.style.setProperty('display', 'block', 'important');
      setTimeout(() => {
        const cv = container.querySelector('canvas');
        if (cv) cv.style.setProperty('display', 'block', 'important');
      }, 30);
    } catch (e) {
      console.warn("Base QRCode error:", e);
    }
  }
}

function setBaseWithdrawAsset(asset) {
  baseWithdrawAsset = asset;
  const btnUsdc = document.getElementById('btn-base-asset-usdc');
  const btnEth = document.getElementById('btn-base-asset-eth');
  const label = document.getElementById('base-withdraw-asset-label');
  const adorn = document.getElementById('base-withdraw-adornment');
  const availLabel = document.getElementById('base-modal-avail-bal');

  if (btnUsdc) btnUsdc.classList.toggle('highlight', asset === 'USDC');
  if (btnEth) btnEth.classList.toggle('highlight', asset === 'ETH');
  if (label) label.innerText = asset;
  if (adorn) adorn.innerText = asset;

  const usdcBal = vaultData ? (vaultData.base_usdc_balance || 0) : 0;
  const ethBal = vaultData ? (vaultData.base_eth_balance || 0) : 0;
  if (availLabel) {
    availLabel.innerText = asset === 'USDC' 
      ? `${usdcBal > 0 ? floorDecimals(usdcBal, 6).toString() : "0.00"} USDC` 
      : `${floorDecimals(ethBal, 6).toFixed(6)} ETH`;
  }

  const inp = document.getElementById('base-withdraw-amount');
  if (inp) inp.value = '';
  fetchBaseGasEstimate();
  validateBaseWithdrawInput();
}

function floorDecimals(num, decimals) {
  if (!num || isNaN(num) || num <= 0) return 0;
  const factor = Math.pow(10, decimals);
  return Math.floor((Number(num) + 1e-12) * factor) / factor;
}

function setBaseWithdrawPercent(pct) {
  if (pct >= 1.0) {
    setBaseWithdrawMax();
    return;
  }
  const amtInput = document.getElementById('base-withdraw-amount');
  if (!amtInput || !vaultData) return;
  if (baseWithdrawAsset === 'USDC') {
    const avail = vaultData.base_usdc_balance || 0;
    amtInput.value = floorDecimals(avail * pct, 6).toString();
  } else {
    const feeToDeduct = currentBaseGasEstimate?.total_fee_eth ? (currentBaseGasEstimate.total_fee_eth * 1.15) : 0.000010;
    const maxEth = Math.max(0, (vaultData.base_eth_balance || 0) - feeToDeduct);
    amtInput.value = floorDecimals(maxEth * pct, 6).toFixed(6);
  }
  validateBaseWithdrawInput();
}

function setBaseWithdrawMax() {
  const inp = document.getElementById('base-withdraw-amount');
  if (!inp || !vaultData) return;
  if (baseWithdrawAsset === 'USDC') {
    const avail = vaultData.base_usdc_balance || 0;
    inp.value = avail > 0 ? floorDecimals(avail, 6).toString() : "0.00";
  } else {
    const feeToDeduct = currentBaseGasEstimate?.total_fee_eth ? (currentBaseGasEstimate.total_fee_eth * 1.15) : 0.000010;
    const maxEth = Math.max(0, (vaultData.base_eth_balance || 0) - feeToDeduct);
    inp.value = maxEth > 0 ? floorDecimals(maxEth, 6).toFixed(6) : "0.000000";
  }
  validateBaseWithdrawInput();
}

function validateBaseWithdrawInput() {
  const destInput = document.getElementById('base-withdraw-dest');
  const amtInput = document.getElementById('base-withdraw-amount');
  const hint = document.getElementById('base-dest-hint');
  const btn = document.getElementById('btn-base-do-withdraw');
  const statusBox = document.getElementById('base-withdraw-status');

  const dest = destInput ? destInput.value.trim() : "";
  const amt = amtInput ? parseFloat(amtInput.value) || 0 : 0;

  let validDest = false;
  if (dest.startsWith('0x') && dest.length === 42) {
    validDest = true;
    if (hint) {
      hint.innerText = "✓ Valid Base EVM Address format";
      hint.style.color = "var(--accent-green)";
    }
  } else if (dest.length > 0) {
    if (hint) {
      hint.innerText = "Invalid Base address (must start with '0x', 42 chars)";
      hint.style.color = "var(--accent-red)";
    }
  } else {
    if (hint) {
      hint.innerText = "Must start with '0x' (42 characters EVM)";
      hint.style.color = "var(--text-dim)";
    }
  }

  const feeToDeduct = currentBaseGasEstimate?.total_fee_eth ? (currentBaseGasEstimate.total_fee_eth * 1.15) : 0.000010;
  const rawAvail = baseWithdrawAsset === 'USDC' 
    ? (vaultData ? (vaultData.base_usdc_balance || 0) : 0)
    : (vaultData ? Math.max(0, (vaultData.base_eth_balance || 0) - feeToDeduct) : 0);

  // Epsilon tolerance for display/input rounding (e.g. typing 2.61 when exact balance is 2.607692)
  const epsilon = baseWithdrawAsset === 'USDC' ? 0.01 : 0.000002;
  const avail = rawAvail + epsilon;

  const ethBal = vaultData ? (vaultData.base_eth_balance || 0) : 0;
  const gasNeeded = currentBaseGasEstimate?.total_fee_eth || 0.000008;
  const hasGasForTx = ethBal >= gasNeeded;

  // Validation check
  const hasEnough = amt > 0 && amt <= avail;
  const canSubmit = validDest && hasEnough && hasGasForTx;
  if (btn) btn.disabled = !canSubmit;

  if (amt > 0 && amt > avail) {
    if (statusBox) {
      statusBox.style.display = 'block';
      statusBox.style.background = 'rgba(239, 68, 68, 0.12)';
      statusBox.style.color = '#f87171';
      statusBox.style.border = '1px solid rgba(239, 68, 68, 0.3)';
      const maxDisplay = baseWithdrawAsset === 'USDC' ? floorDecimals(rawAvail, 6) : floorDecimals(rawAvail, 6).toFixed(6);
      statusBox.innerText = `⚠️ Capped to available ${baseWithdrawAsset} balance: ${maxDisplay} ${baseWithdrawAsset}`;
    }
  } else if (amt > 0 && !hasGasForTx) {
    if (statusBox) {
      statusBox.style.display = 'block';
      statusBox.style.background = 'rgba(239, 68, 68, 0.12)';
      statusBox.style.color = '#f87171';
      statusBox.style.border = '1px solid rgba(239, 68, 68, 0.3)';
      statusBox.innerText = `⚠️ Insufficient ETH balance to cover real network gas (~${gasNeeded.toFixed(6)} ETH required, vault has ${ethBal.toFixed(6)} ETH).`;
    }
  } else if (statusBox && (statusBox.innerText.includes('Insufficient') || statusBox.innerText.includes('⚠️') || statusBox.innerText.includes('Capped'))) {
    statusBox.style.display = 'none';
  }
}

async function submitBaseWithdraw() {
  const destEl = document.getElementById('base-withdraw-dest');
  const amtEl = document.getElementById('base-withdraw-amount');
  const btn = document.getElementById('btn-base-do-withdraw');
  const statusBox = document.getElementById('base-withdraw-status');

  const dest = destEl ? destEl.value.trim() : '';
  const amt = amtEl ? parseFloat(amtEl.value) : 0;

  if (!dest || !dest.startsWith('0x') || dest.length !== 42) {
    const msg = "⚠️ Please enter a valid recipient 0x... EVM Base address.";
    if (typeof showToast === 'function') showToast(msg, "warning");
    if (statusBox) {
      statusBox.style.background = 'rgba(239, 68, 68, 0.12)';
      statusBox.style.color = '#f87171';
      statusBox.style.border = '1px solid rgba(239, 68, 68, 0.3)';
      statusBox.innerText = msg;
    }
    return;
  }
  if (!amt || isNaN(amt) || amt <= 0) {
    const msg = "⚠️ Please enter a valid amount greater than 0.";
    if (typeof showToast === 'function') showToast(msg, "warning");
    if (statusBox) {
      statusBox.style.background = 'rgba(239, 68, 68, 0.12)';
      statusBox.style.color = '#f87171';
      statusBox.style.border = '1px solid rgba(239, 68, 68, 0.3)';
      statusBox.innerText = msg;
    }
    return;
  }

  const rawAvail = baseWithdrawAsset === 'USDC'
    ? (vaultData ? (vaultData.base_usdc_balance || 0) : 0)
    : (vaultData ? (vaultData.base_eth_balance || 0) : 0);

  const epsilon = baseWithdrawAsset === 'USDC' ? 0.01 : 0.000002;
  if (amt > (rawAvail + epsilon)) {
    const msg = `⚠️ Insufficient ${baseWithdrawAsset} balance. Have: ${rawAvail} ${baseWithdrawAsset}, Requested: ${amt} ${baseWithdrawAsset}`;
    if (typeof showToast === 'function') showToast(msg, "warning");
    if (statusBox) {
      statusBox.style.background = 'rgba(239, 68, 68, 0.12)';
      statusBox.style.color = '#f87171';
      statusBox.style.border = '1px solid rgba(239, 68, 68, 0.3)';
      statusBox.innerText = msg;
    }
    return;
  }

  // Safely clamp actual broadcast amount to exact raw balance if user rounded up slightly
  const actualSendAmt = Math.min(amt, rawAvail);

  if (btn) {
    btn.disabled = true;
    btn.innerText = "Broadcasting EVM transaction...";
  }
  if (statusBox) {
    statusBox.style.display = 'block';
    statusBox.style.background = 'rgba(0, 82, 255, 0.12)';
    statusBox.style.color = '#38bdf8';
    statusBox.style.border = '1px solid rgba(0, 82, 255, 0.3)';
    statusBox.innerText = `⏳ Signing & broadcasting ${baseWithdrawAsset} withdrawal on ${BASE_NETWORK_NAME}...`;
    setTimeout(() => {
      try { statusBox.scrollIntoView({ behavior: 'smooth', block: 'nearest' }); } catch (e) {}
    }, 50);
  }

  try {
    if (typeof sendSocketAction !== 'function') {
      throw new Error("WebSocket connection is not active");
    }
    const data = await sendSocketAction('withdraw_base', {
      asset: baseWithdrawAsset,
      destination: dest,
      amount: actualSendAmt
    });

    if (!data || (data.status && data.status !== 'ok')) {
      throw new Error((data && (data.message || data.error)) || "Withdrawal failed");
    }

    if (statusBox) {
      statusBox.style.background = 'rgba(16, 185, 129, 0.12)';
      statusBox.style.color = '#34d399';
      statusBox.style.border = '1px solid rgba(16, 185, 129, 0.3)';
      statusBox.innerHTML = `✓ ${baseWithdrawAsset} Withdrawal broadcasted! TX: <a href="${BASE_EXPLORER_URL}/tx/${data.tx_hash}" target="_blank" style="color: #38bdf8; text-decoration: underline;">${data.tx_hash.slice(0, 14)}...</a>`;
      setTimeout(() => {
        try { statusBox.scrollIntoView({ behavior: 'smooth', block: 'nearest' }); } catch (e) {}
      }, 50);
    }

    if (typeof showToast === 'function') {
      showToast(`🎉 Broadcasted ${amt} ${baseWithdrawAsset} to ${dest.slice(0, 8)}...`);
    }

    if (amtEl) amtEl.value = '';
    if (destEl) destEl.value = '';
    fetchVaultStatus(true);

  } catch (e) {
    console.error("Base withdraw error:", e);
    if (statusBox) {
      statusBox.style.background = 'rgba(239, 68, 68, 0.12)';
      statusBox.style.color = '#f87171';
      statusBox.style.border = '1px solid rgba(239, 68, 68, 0.3)';
      statusBox.innerText = `⚠️ Withdrawal error: ${e.message}`;
      setTimeout(() => {
        try { statusBox.scrollIntoView({ behavior: 'smooth', block: 'nearest' }); } catch (e) {}
      }, 50);
    }
    if (typeof showToast === 'function') {
      showToast(`⚠️ Withdrawal failed: ${e.message}`);
    }
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.innerText = "Broadcast Base Withdrawal";
    }
  }
}

// --- Base Key Export ---
async function fetchBaseExport() {
  const container = document.getElementById('base-export-content-container');
  const errorBox = document.getElementById('base-export-error-box');
  const loadingBox = document.getElementById('base-export-loading-box');

  if (vaultExportData && vaultExportData.base_priv_key_hex) {
    if (loadingBox) loadingBox.style.display = 'none';
    if (container) container.style.display = 'block';
    if (errorBox) errorBox.style.display = 'none';
    renderBaseExport();
    return;
  }

  if (loadingBox) loadingBox.style.display = 'block';
  if (container) container.style.display = 'none';
  if (errorBox) errorBox.style.display = 'none';

  if (typeof requestVaultExportOverSocket === 'function' && ws && ws.readyState === WebSocket.OPEN) {
    const tok = sessionToken || window.SERVER_SESSION_TOKEN || "";
    if (requestVaultExportOverSocket(tok)) {
      return;
    }
  }

  if (loadingBox) loadingBox.style.display = 'none';
  if (container) container.style.display = 'none';
  if (errorBox) {
    errorBox.style.display = 'block';
    errorBox.innerText = "Base export error: WebSocket connection is not open";
  }
}

function renderBaseExport() {
  if (!vaultExportData) return;
  const baseAddrEl = document.getElementById('base-export-addr-val');
  if (baseAddrEl) baseAddrEl.innerText = vaultExportData.base_address || (vaultData ? vaultData.base_address : 'N/A');

  const privEl = document.getElementById('base-export-priv-val');
  const btn = document.getElementById('btn-toggle-base-priv-modal');
  if (privEl && btn) {
    if (baseExportRevealed.priv) {
      privEl.innerText = vaultExportData.base_priv_key_hex;
      privEl.classList.remove('export-blur');
      btn.innerText = "🔒 Hide";
    } else {
      privEl.innerText = "••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••";
      privEl.classList.add('export-blur');
      btn.innerText = "👁 Show";
    }
  }

  const mnemEl = document.getElementById('base-export-mnemonic-val');
  const btnMnem = document.getElementById('btn-toggle-base-mnemonic');
  if (mnemEl && btnMnem) {
    if (baseExportRevealed.mnemonic) {
      mnemEl.innerText = vaultExportData.base_mnemonic || vaultExportData.mnemonic || 'N/A';
      mnemEl.classList.remove('export-blur');
      btnMnem.innerText = "🔒 Hide Words";
    } else {
      mnemEl.innerText = "•••• •••• •••• •••• •••• •••• •••• •••• •••• •••• •••• ••••";
      mnemEl.classList.add('export-blur');
      btnMnem.innerText = "👁 Show Words";
    }
  }
}

function toggleBaseExportReveal(field) {
  if (field === 'priv') {
    baseExportRevealed.priv = !baseExportRevealed.priv;
    renderBaseExport();
  } else if (field === 'mnemonic') {
    baseExportRevealed.mnemonic = !baseExportRevealed.mnemonic;
    renderBaseExport();
  }
}

function copyBaseExportField(field, toastMsg) {
  let text = "";
  if (field === 'address') {
    text = (vaultExportData && vaultExportData.base_address) ? vaultExportData.base_address : (vaultData ? vaultData.base_address : "");
  } else if (field === 'priv') {
    text = vaultExportData ? vaultExportData.base_priv_key_hex : "";
  } else if (field === 'mnemonic') {
    text = vaultExportData ? (vaultExportData.base_mnemonic || vaultExportData.mnemonic) : "";
  }
  if (text) {
    copyToClipboard(text, toastMsg || "✓ Copied to clipboard!");
  }
}

// --- Destination Address Helpers & QR Code Scanner ---

function cleanCryptoAddress(raw) {
  if (!raw) return '';
  let str = String(raw).trim();
  str = str.replace(/^["'`]|["'`]$/g, '');
  str = str.replace(/^(tru|ethereum|eth|base|web3):\/\//i, '');
  str = str.replace(/^(tru|ethereum|eth|base|web3):/i, '');
  if (str.includes('?')) {
    str = str.split('?')[0];
  }
  if (str.includes('@')) {
    str = str.split('@')[0];
  }
  return str.trim();
}

async function pasteAddress(inputId) {
  const input = document.getElementById(inputId);
  if (!input) return;

  let text = '';

  // 1. First attempt: Native Android Clipboard bridge via WebSocket (no permission popups/alerts)
  if (typeof sendSocketAction === 'function') {
    try {
      const res = await sendSocketAction('get_clipboard');
      if (res && res.text) {
        text = String(res.text).trim();
      }
    } catch (e) {
      console.warn("Native get_clipboard via socket failed:", e);
    }
  }

  // 2. Second attempt: Standard Async Clipboard API (for desktop browsers)
  if (!text && navigator.clipboard && navigator.clipboard.readText) {
    try {
      text = (await navigator.clipboard.readText() || '').trim();
    } catch (err) {
      console.warn("navigator.clipboard.readText blocked or unavailable:", err);
    }
  }

  if (!text) {
    showToast("Clipboard is empty. Copy an address first!", "warning");
    input.focus();
    return;
  }

  const cleaned = cleanCryptoAddress(text);
  if (!cleaned) {
    showToast("No valid address found in clipboard", "warning");
    input.focus();
    return;
  }

  input.value = cleaned;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.focus();
  const short = cleaned.length > 14 ? `${cleaned.slice(0, 6)}...${cleaned.slice(-4)}` : cleaned;
  showToast(`✓ Pasted: ${short}`, 'success');
}

var _qrState = {
  active: false,
  targetInputId: null,
  stream: null,
  animId: null,
  facingMode: 'environment',
  torch: false,
  track: null,
  barcodeDetector: null,
};

function playQrBeep() {
  try {
    const AudioCtx = window.AudioContext || window.webkitAudioContext;
    if (!AudioCtx) return;
    const ctx = new AudioCtx();
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = 'sine';
    osc.frequency.setValueAtTime(880, ctx.currentTime);
    osc.frequency.exponentialRampToValueAtTime(1320, ctx.currentTime + 0.08);
    gain.gain.setValueAtTime(0.12, ctx.currentTime);
    gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + 0.1);
    osc.connect(gain);
    gain.connect(ctx.destination);
    osc.start();
    osc.stop(ctx.currentTime + 0.1);
  } catch (e) {
    // Audio context may be restricted or unsupported
  }
}

function stopQrCameraTracks() {
  if (_qrState.animId) {
    cancelAnimationFrame(_qrState.animId);
    _qrState.animId = null;
  }
  if (_qrState.stream) {
    try {
      _qrState.stream.getTracks().forEach(t => t.stop());
    } catch (e) {}
    _qrState.stream = null;
  }
  _qrState.track = null;
  _qrState.torch = false;
  const video = document.getElementById('qr-video');
  if (video) {
    video.srcObject = null;
    video.style.display = 'block';
  }
  const streamImg = document.getElementById('qr-camera-stream');
  if (streamImg) {
    streamImg.style.display = 'none';
    streamImg.src = '';
  }
}

async function startQrCamera() {
  stopQrCameraTracks();
  const video = document.getElementById('qr-video');
  const loadingEl = document.getElementById('qr-loading-overlay');
  const errorEl = document.getElementById('qr-error-overlay');
  const torchBtn = document.getElementById('qr-btn-torch');
  const vf = document.querySelector('.qr-viewfinder');

  if (vf) vf.classList.remove('qr-success-pulse');
  if (loadingEl) loadingEl.style.display = 'flex';
  if (errorEl) errorEl.style.display = 'none';

  const isAndroid = /android/i.test(navigator.userAgent || '');
  if (isAndroid) {
    // Pure native Rust NDK Camera2 scanner on Android
    _qrState.active = true;
    if (typeof sendSocketAction === 'function') {
      sendSocketAction('start_qr_scan', { facing: _qrState.facingMode });
    }
    return;
  }

  try {
    const constraints = {
      video: {
        facingMode: { ideal: _qrState.facingMode },
        width: { ideal: 1280 },
        height: { ideal: 720 },
      },
      audio: false,
    };
    const stream = await navigator.mediaDevices.getUserMedia(constraints);
    _qrState.stream = stream;
    _qrState.track = stream.getVideoTracks()[0] || null;

    if (_qrState.track && typeof _qrState.track.getCapabilities === 'function') {
      const caps = _qrState.track.getCapabilities();
      if (caps && caps.torch) {
        if (torchBtn) torchBtn.style.display = 'inline-flex';
      } else {
        if (torchBtn) torchBtn.style.display = 'none';
      }
    } else {
      if (torchBtn) torchBtn.style.display = 'none';
    }

    if (video) {
      video.srcObject = stream;
      await video.play();
    }

    if (loadingEl) loadingEl.style.display = 'none';
    _qrState.active = true;
    _qrState.animId = requestAnimationFrame(qrScanLoop);
  } catch (err) {
    console.warn("Browser camera start failed, trying native camera:", err);
    if (typeof sendSocketAction === 'function') {
      sendSocketAction('start_qr_scan');
    } else {
      if (loadingEl) loadingEl.style.display = 'none';
      if (errorEl) {
        errorEl.style.display = 'flex';
        const errText = document.getElementById('qr-error-text');
        if (errText) {
          errText.innerText = "Camera unavailable. You can upload a QR image below.";
        }
      }
    }
  }
}

async function qrScanLoop() {
  if (!_qrState.active) return;
  const video = document.getElementById('qr-video');
  const canvas = document.getElementById('qr-canvas');
  if (!video || !canvas) return;

  if (video.readyState === video.HAVE_ENOUGH_DATA) {
    let code = null;

    // 1. Try native BarcodeDetector if available
    if (_qrState.barcodeDetector) {
      try {
        const barcodes = await _qrState.barcodeDetector.detect(video);
        if (barcodes && barcodes.length > 0 && barcodes[0].rawValue) {
          code = barcodes[0].rawValue;
        }
      } catch (e) {
        // Fallback to jsQR
      }
    }

    // 2. Fallback to jsQR on canvas
    if (!code && window.jsQR) {
      try {
        const ctx = canvas.getContext('2d', { willReadFrequently: true });
        if (canvas.width !== video.videoWidth || canvas.height !== video.videoHeight) {
          canvas.width = video.videoWidth || 640;
          canvas.height = video.videoHeight || 480;
        }
        ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
        const imgData = ctx.getImageData(0, 0, canvas.width, canvas.height);
        const result = window.jsQR(imgData.data, imgData.width, imgData.height, {
          inversionAttempts: "dontInvert",
        });
        if (result && result.data) {
          code = result.data;
        }
      } catch (e) {}
    }

    if (code) {
      onQrSuccess(code);
      return;
    }
  }

  if (_qrState.active) {
    _qrState.animId = requestAnimationFrame(qrScanLoop);
  }
}

function onQrSuccess(rawCode) {
  if (!_qrState.active) return;
  _qrState.active = false;

  playQrBeep();

  const cleaned = cleanCryptoAddress(rawCode);
  const targetInput = document.getElementById(_qrState.targetInputId);
  if (targetInput) {
    targetInput.value = cleaned;
    targetInput.dispatchEvent(new Event('input', { bubbles: true }));
    targetInput.dispatchEvent(new Event('change', { bubbles: true }));
    targetInput.focus();
  }

  // Close scanner immediately as requested by user
  closeQrScanner();

  const short = cleaned.length > 14 ? `${cleaned.slice(0, 6)}...${cleaned.slice(-4)}` : cleaned;
  showToast(`✓ Scanned: ${short}`, 'success');
}

function openQrScanner(targetInputId, networkLabel) {
  _qrState.targetInputId = targetInputId;
  _qrState.active = true;
  const badge = document.getElementById('qr-target-badge');
  if (badge) badge.innerText = networkLabel || 'Address';

  const modal = document.getElementById('qr-scanner-modal');
  if (modal) modal.style.display = 'flex';

  if ('BarcodeDetector' in window) {
    try {
      _qrState.barcodeDetector = new BarcodeDetector({ formats: ['qr_code'] });
    } catch (e) {
      _qrState.barcodeDetector = null;
    }
  } else {
    _qrState.barcodeDetector = null;
  }

  startQrCamera();
}

function closeQrScanner() {
  _qrState.active = false;
  stopQrCameraTracks();
  if (typeof sendSocketAction === 'function') {
    sendSocketAction('stop_qr_scan');
  }
  const modal = document.getElementById('qr-scanner-modal');
  if (modal) modal.style.display = 'none';
  const fileInput = document.getElementById('qr-file-input');
  if (fileInput) fileInput.value = '';
}

function retryQrCamera() {
  startQrCamera();
}

function flipQrCamera() {
  _qrState.facingMode = (_qrState.facingMode === 'environment') ? 'user' : 'environment';
  const flipBtn = document.getElementById('qr-btn-flip');
  if (flipBtn) {
    flipBtn.classList.toggle('active', _qrState.facingMode === 'user');
  }
  startQrCamera();
}

async function toggleQrTorch() {
  if (!_qrState.track) return;
  try {
    _qrState.torch = !_qrState.torch;
    await _qrState.track.applyConstraints({
      advanced: [{ torch: _qrState.torch }]
    });
    const torchBtn = document.getElementById('qr-btn-torch');
    if (torchBtn) {
      torchBtn.classList.toggle('active', _qrState.torch);
    }
  } catch (e) {
    console.warn("Torch toggle failed:", e);
  }
}

function onQrFileSelected(event) {
  const file = event.target.files && event.target.files[0];
  if (!file) return;

  const reader = new FileReader();
  reader.onload = function(e) {
    const img = new Image();
    img.onload = async function() {
      const canvas = document.getElementById('qr-canvas') || document.createElement('canvas');
      canvas.width = img.width;
      canvas.height = img.height;
      const ctx = canvas.getContext('2d');
      ctx.drawImage(img, 0, 0);

      let code = null;

      // 1. Try BarcodeDetector
      if (_qrState.barcodeDetector) {
        try {
          const barcodes = await _qrState.barcodeDetector.detect(canvas);
          if (barcodes && barcodes.length > 0 && barcodes[0].rawValue) {
            code = barcodes[0].rawValue;
          }
        } catch (err) {}
      }

      // 2. Try jsQR
      if (!code && window.jsQR) {
        try {
          const imgData = ctx.getImageData(0, 0, canvas.width, canvas.height);
          const result = window.jsQR(imgData.data, imgData.width, imgData.height, {
            inversionAttempts: "attemptBoth",
          });
          if (result && result.data) {
            code = result.data;
          }
        } catch (err) {}
      }

      if (code) {
        onQrSuccess(code);
      } else {
        showToast("No QR code found in this image", "warning");
      }
    };
    img.src = e.target.result;
  };
  reader.readAsDataURL(file);
}

function onQrModalBackdropClick(event) {
  if (event.target && event.target.id === 'qr-scanner-modal') {
    closeQrScanner();
  }
}

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && _qrState.active) {
    closeQrScanner();
  }
});

// Global exports for inline HTML onclick handlers
window.openVaultModal = openVaultModal;
window.closeVaultModal = closeVaultModal;
window.switchVaultTab = switchVaultTab;
window.copyTruAddress = copyTruAddress;
window.copyBaseAddress = copyBaseAddress;
window.setMaxWithdraw = setMaxWithdraw;
window.setWithdrawPercent = setWithdrawPercent;
window.submitVaultWithdraw = submitVaultWithdraw;
window.copyExportField = copyExportField;
window.toggleExportReveal = toggleExportReveal;
window.openBaseModal = openBaseModal;
window.closeBaseModal = closeBaseModal;
window.switchBaseTab = switchBaseTab;
window.renderBaseQrCode = renderBaseQrCode;
window.setBaseWithdrawAsset = setBaseWithdrawAsset;
window.setBaseWithdrawPercent = setBaseWithdrawPercent;
window.setBaseWithdrawMax = setBaseWithdrawMax;
window.validateBaseWithdrawInput = validateBaseWithdrawInput;
window.submitBaseWithdraw = submitBaseWithdraw;
window.fetchBaseExport = fetchBaseExport;
window.renderBaseExport = renderBaseExport;
window.toggleBaseExportReveal = toggleBaseExportReveal;
window.copyBaseExportField = copyBaseExportField;
window.fetchBaseGasEstimate = fetchBaseGasEstimate;

// Export QR and Paste functions
window.cleanCryptoAddress = cleanCryptoAddress;
window.pasteAddress = pasteAddress;
window.openQrScanner = openQrScanner;
window.closeQrScanner = closeQrScanner;
window.onQrSuccess = onQrSuccess;
window.retryQrCamera = retryQrCamera;
window.flipQrCamera = flipQrCamera;
window.toggleQrTorch = toggleQrTorch;
window.onQrFileSelected = onQrFileSelected;
window.onQrModalBackdropClick = onQrModalBackdropClick;


