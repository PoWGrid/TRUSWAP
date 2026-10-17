// --- Dual On-Chain Orderbook & Swaps Ledger ---

// --- My Orders Tracking (Global) ---
const myCreatedOrders = new Set();
try {
  const saved = JSON.parse(localStorage.getItem('my_truswap_orders') || '[]');
  saved.forEach(id => myCreatedOrders.add(id));
} catch (e) { }

function addMyCreatedOrder(hash160, id) {
  if (hash160) myCreatedOrders.add(hash160);
  if (id) myCreatedOrders.add(id);
  try {
    localStorage.setItem('my_truswap_orders', JSON.stringify(Array.from(myCreatedOrders)));
  } catch (e) { }
}
window.addMyCreatedOrder = addMyCreatedOrder;

// --- Claimed Base Swaps Tracking ---
const claimedBaseSwaps = new Set();
try {
  const savedClaimed = JSON.parse(localStorage.getItem('claimed_base_swaps') || '[]');
  savedClaimed.forEach(id => claimedBaseSwaps.add(id.toLowerCase()));
} catch (e) { }
// Seed known claimed swap on Base
claimedBaseSwaps.add("4ad8e6f5d9d915366582e259d673b09e021067f3");
claimedBaseSwaps.add("0x9a06242094ddc9b64fa0bb620dc827b7077a32fbfd574f209dd265c883ad0446");
claimedBaseSwaps.add("9a06242094ddc9b64fa0bb620dc827b7077a32fbfd574f209dd265c883ad0446");
window.claimedBaseSwaps = claimedBaseSwaps;

function markBaseSwapAsClaimed(id, hash160) {
  if (id) {
    const cId = id.toLowerCase().replace(/^0x/, '');
    claimedBaseSwaps.add(cId);
    claimedBaseSwaps.add('0x' + cId);
  }
  if (hash160) {
    const cH = hash160.toLowerCase().replace(/^0x/, '');
    claimedBaseSwaps.add(cH);
    claimedBaseSwaps.add('0x' + cH);
  }
  try {
    localStorage.setItem('claimed_base_swaps', JSON.stringify(Array.from(claimedBaseSwaps)));
  } catch (e) { }
}
window.markBaseSwapAsClaimed = markBaseSwapAsClaimed;

const checkedBaseSwaps = new Set();
async function checkBaseSwapClaimed(swapId, hash160) {
  if (!swapId || swapId.length < 64) return;
  const cleanId = swapId.toLowerCase().replace(/^0x/, '');
  if (checkedBaseSwaps.has(cleanId)) return;
  checkedBaseSwaps.add(cleanId);
  try {
    const rpcEndpoint = (typeof BASE_CHAIN_ID !== 'undefined' && BASE_CHAIN_ID === 84532) ? 'https://sepolia.base.org' : 'https://mainnet.base.org';
    const res = await fetch(rpcEndpoint, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'eth_call',
        params: [{
          to: typeof BASE_HTLC_ADDR !== 'undefined' ? BASE_HTLC_ADDR : '0xF2e70fbd7Da79d188f805e61e8587AACD49772a5',
          data: '0x3da0e66e' + cleanId.padStart(64, '0')
        }, 'latest']
      })
    });
    const json = await res.json();
    if (json.result && json.result.length >= 66) {
      const data = json.result.slice(2);
      if (data.length >= 7 * 64) {
        const sender = data.slice(24, 64);
        const claimedWord = data.slice(6 * 64, 7 * 64);
        const isClaimed = parseInt(claimedWord, 16) === 1;
        const isNonExistent = sender === '0000000000000000000000000000000000000000';
        if (isClaimed || isNonExistent) {
          markBaseSwapAsClaimed(cleanId, hash160);
          if (typeof renderTradeHistory === 'function') renderTradeHistory();
        }
      }
    }
  } catch (e) {
    console.warn('Check Base claimed error:', e);
  }
}
window.checkBaseSwapClaimed = checkBaseSwapClaimed;

function isMyOrder(o) {
  if (!o) return false;
  const cleanHash = (o.hash160 || '').replace(/^0x/, '').toLowerCase();
  const cleanId = (o.id || '').replace(/^0x/, '').toLowerCase();
  if (cleanHash && (myCreatedOrders.has(cleanHash) || myCreatedOrders.has('0x' + cleanHash) || myCreatedOrders.has(o.hash160))) return true;
  if (cleanId && (myCreatedOrders.has(cleanId) || myCreatedOrders.has('0x' + cleanId) || myCreatedOrders.has(o.id))) return true;
  if (o.makerBaseAddress && typeof userBaseAddress !== 'undefined' && userBaseAddress && o.makerBaseAddress.toLowerCase() === userBaseAddress.toLowerCase()) return true;
  if (o.makerTruPubkey && typeof vaultData !== 'undefined' && vaultData && vaultData.pubkey && o.makerTruPubkey.toLowerCase() === vaultData.pubkey.toLowerCase()) return true;
  if (o.makerTruAddress && typeof vaultData !== 'undefined' && vaultData && vaultData.address && o.makerTruAddress === vaultData.address) return true;
  return false;
}
window.isMyOrder = isMyOrder;

async function cancelOrder(orderId, hash160) {
  const cleanId = (orderId && orderId !== 'undefined' && orderId !== 'null') ? String(orderId).replace(/^0x/, '').toLowerCase() : null;
  const cleanHash = (hash160 && hash160 !== 'undefined' && hash160 !== 'null') ? String(hash160).replace(/^0x/, '').toLowerCase() : null;
  const targetId = cleanId || cleanHash;

  // Immediate visual feedback: purge from ordersMap and myCreatedOrders right away
  const idsToPurge = [cleanId, cleanHash].filter(Boolean);
  for (const [k, v] of Array.from(ordersMap.entries())) {
    const vId = (v.id || '').replace(/^0x/, '').toLowerCase();
    const vHash = (v.hash160 || '').replace(/^0x/, '').toLowerCase();
    if (idsToPurge.includes(k.toLowerCase()) || idsToPurge.includes(vId) || idsToPurge.includes(vHash)) {
      ordersMap.delete(k);
    }
  }
  for (const item of Array.from(myCreatedOrders)) {
    const cItem = item.replace(/^0x/, '').toLowerCase();
    if (idsToPurge.includes(cItem)) {
      myCreatedOrders.delete(item);
    }
  }
  try {
    localStorage.setItem('my_truswap_orders', JSON.stringify(Array.from(myCreatedOrders)));
  } catch (e) { }

  renderOrderbook();
  renderActiveSwapsTracker();
  if (typeof showToast === 'function') showToast("✓ Cancelling order...");

  // Send cancellation to local node over WebSocket
  let cancelData = null;
  try {
    cancelData = await sendSocketAction('cancel_order', { id: targetId, hash160: cleanHash || cleanId });
  } catch (e) {
    console.warn("Local node cancel request error:", e);
  }

  if (typeof showToast === 'function') showToast("✓ Order cancelled successfully");
  renderOrderbook();
  renderActiveSwapsTracker();
  if (typeof fetchHistoricalLedgerData === 'function') {
    fetchHistoricalLedgerData();
  }
}
window.cancelOrder = cancelOrder;

function formatRemainingTime(timelockSec) {
  if (!timelockSec || timelockSec <= 0) return '—';
  const nowSec = Math.floor(Date.now() / 1000);
  const diffSec = timelockSec - nowSec;
  const fullDateStr = new Date(timelockSec * 1000).toLocaleString();

  if (diffSec <= 0) {
    return `<span style="color: var(--accent-red); font-weight: 600;" title="Expired at: ${fullDateStr}">Expired</span>`;
  }

  const hours = Math.floor(diffSec / 3600);
  const minutes = Math.floor((diffSec % 3600) / 60);

  let formatted = '';
  if (hours > 0) {
    formatted = `${hours}h ${minutes}m`;
  } else {
    formatted = `${minutes}m`;
  }

  const colorStyle = hours < 2
    ? 'color: var(--tru-gold); font-weight: 600;'
    : 'color: #94a3b8;';

  return `<span style="${colorStyle}" title="Expires at: ${fullDateStr}">${formatted}</span>`;
}

function updateTableHeaders(filter) {
  const tr = document.getElementById('orderbook-headers');
  const colgroup = document.getElementById('orderbook-colgroup');
  const table = document.getElementById('orderbook-table');
  if (!tr) return;

  if (filter === 'mempool') {
    if (table) table.style.minWidth = '660px';
    if (colgroup) {
      colgroup.innerHTML = `
        <col style="width: 110px;">
        <col style="width: 80px;">
        <col style="width: 90px;">
        <col style="width: 80px;">
        <col style="width: 80px;">
        <col style="width: 70px;">
        <col style="width: 85px;">
        <col style="width: 95px;">
      `;
    }
    tr.innerHTML = `
      <th style="text-align: left; padding-left: 10px;">Type</th>
      <th>Channel</th>
      <th style="text-align: right;">Amount</th>
      <th>Outputs</th>
      <th>Inputs</th>
      <th>Age</th>
      <th>TXID</th>
      <th>Status</th>
    `;
  } else if (filter === 'inprogress') {
    if (table) table.style.minWidth = '580px';
    if (colgroup) {
      colgroup.innerHTML = `
        <col style="width: 70px;">
        <col style="width: 170px;">
        <col style="width: 95px;">
        <col style="width: 95px;">
        <col style="width: 75px;">
        <col style="width: 75px;">
      `;
    }
    tr.innerHTML = `
      <th style="text-align: left; padding-left: 10px;">Side</th>
      <th>Network Confirmations</th>
      <th style="text-align: right;">Amount (TRU)</th>
      <th style="text-align: right;">Total (USDC)</th>
      <th>Expires</th>
      <th style="text-align: center;">Action</th>
    `;
  } else if (filter === 'history' || filter === 'ledger') {
    if (table) table.style.minWidth = '100%';
    if (colgroup) {
      colgroup.innerHTML = `
        <col style="width: 120px;">
        <col style="width: 65px;">
        <col style="width: 85px;">
        <col style="width: 85px;">
        <col style="width: 85px;">
        <col style="width: 95px;">
        <col style="width: 95px;">
      `;
    }
    tr.innerHTML = `
      <th style="text-align: left; padding-left: 10px;">Block / Time</th>
      <th>Side</th>
      <th style="text-align: right;">Price (USDC)</th>
      <th style="text-align: right;">Amount (TRU)</th>
      <th style="text-align: right;">Total (USDC)</th>
      <th>Settlement</th>
      <th>Claim TXID</th>
    `;
  } else {
    // book, asks, bids
    if (table) table.style.minWidth = '100%';
    if (colgroup) {
      colgroup.innerHTML = `
        <col style="width: 28%;">
        <col style="width: 26%;">
        <col style="width: 26%;">
        <col style="width: 20%;">
      `;
    }
    tr.innerHTML = `
      <th style="text-align: left; padding-left: 10px;">Price (USDC)</th>
      <th style="text-align: right;">Amount (TRU)</th>
      <th style="text-align: right;">Total (USDC)</th>
      <th style="text-align: center;">Action</th>
    `;
  }
}

function renderOrderbook() {
  const tbody = document.getElementById('orderbook-rows');
  if (!tbody) return;

  if (currentFilter === 'mempool') {
    renderMempoolRows(mempoolData.transactions);
    fetchMempoolData();
    return;
  }
  if (currentFilter === 'history' || currentFilter === 'ledger') {
    renderTradeHistory();
    return;
  }
  if (currentFilter === 'inprogress') {
    renderInProgressRows();
    return;
  }

  updateTableHeaders(currentFilter);

  // Extract and process deduplicated orders
  const seenOrderKeys = new Set();
  const allOrders = [];

  const activeSwaps = getActiveInProgressSwaps();
  const inProgressHashes = new Set(activeSwaps.map(s => (s.hash160_hex || '').replace(/^0x/, '').toLowerCase()));
  const inProgressIds = new Set(activeSwaps.map(s => (s.txid || s.base_swap_id || s.id || '').replace(/^0x/, '').toLowerCase()));

  for (const o of ordersMap.values()) {
    if (!o) continue;
    // The public orderbook displays active open live orders only (not matched, not in progress)
    if (!o.isLive || o.isMatched) continue;
    const cleanHash = (o.hash160 || '').replace(/^0x/, '').toLowerCase();
    const cleanId = (o.id || '').replace(/^0x/, '').toLowerCase();
    if (inProgressHashes.has(cleanHash) || inProgressIds.has(cleanId)) continue;
    if (cleanHash && typeof historicalLedgerData !== 'undefined' && historicalLedgerData && historicalLedgerData.swaps) {
      const isSettled = historicalLedgerData.swaps.some(s => {
        const h = (s.hash160_hex || '').replace(/^0x/, '').toLowerCase();
        return h === cleanHash && (s.status === 'Claimed' || s.status === 'Refunded' || s.is_spent || s.base_claimed || s.vault_status === 1 || s.vault_status === 2);
      });
      if (isSettled) continue;
    }
    const uid = cleanHash || o.id;
    if (!uid || seenOrderKeys.has(uid)) continue;
    seenOrderKeys.add(uid);
    allOrders.push(o);
  }
  const asks = [];
  const bids = [];

  allOrders.forEach(o => {
    const tru = Number(o.truAmount) || 0;
    const usdc = Number(o.usdcAmount) || 0;
    const price = (tru > 0 && usdc > 0) ? (usdc / tru) : (o.price || 1.0);
    const total = usdc > 0 ? usdc : (price * tru);

    const processed = {
      ...o,
      truAmount: tru,
      usdcAmount: usdc,
      price: price,
      total: total
    };

    if (o.side === 'SELL_TRU') {
      asks.push(processed);
    } else {
      bids.push(processed);
    }
  });

  // Sort Asks ascending (lowest price first for calculation of cumulative volume)
  const asksAsc = [...asks].sort((a, b) => a.price - b.price);
  let cumAsk = 0;
  asksAsc.forEach(o => {
    cumAsk += o.truAmount;
    o.cumAmount = cumAsk;
  });
  const totalAskVol = cumAsk > 0 ? cumAsk : 1.0;

  // Sort Bids descending (highest price first for calculation of cumulative volume)
  const bidsDesc = [...bids].sort((a, b) => b.price - a.price);
  let cumBid = 0;
  bidsDesc.forEach(o => {
    cumBid += o.truAmount;
    o.cumAmount = cumBid;
  });
  const totalBidVol = cumBid > 0 ? cumBid : 1.0;

  // Calculate market spread
  const bestAsk = asksAsc.length > 0 ? asksAsc[0].price : 0;
  const bestBid = bidsDesc.length > 0 ? bidsDesc[0].price : 0;
  let spreadStr = "—";
  let lastPriceStr = "$1.0000";

  if (bestAsk > 0 && bestBid > 0) {
    const diff = (bestAsk - bestBid);
    const pct = ((diff / bestAsk) * 100).toFixed(2);
    spreadStr = `$${diff.toFixed(4)} (${pct}%)`;
    lastPriceStr = `$${bestAsk.toFixed(4)}`;
  } else if (bestAsk > 0) {
    lastPriceStr = `$${bestAsk.toFixed(4)}`;
  } else if (bestBid > 0) {
    lastPriceStr = `$${bestBid.toFixed(4)}`;
  }

  const renderAskRow = (o) => {
    const depthPct = Math.min(100, Math.round((o.cumAmount / totalAskVol) * 100));
    const isMine = isMyOrder(o);
    let actionBtn = "";
    if (isMine) {
      if (o.isFundedOnBase) {
        actionBtn = `<button class="ob-btn-trade ob-btn-claim" onclick="claimBaseHtlc('${o.baseSwapId || o.id}', '${o.hash160}')" title="Taker funded ${o.usdcAmount.toFixed(2)} USDC! Click to claim">Claim</button>`;
      } else if (o.isMatched) {
        actionBtn = `<span class="ob-status-capsule" style="background: rgba(245, 158, 11, 0.2); color: #f59e0b; border: 1px solid #f59e0b; padding: 2px 6px; border-radius: 4px; font-size: 0.7rem; font-weight: 700;">IN PROGRESS</span>`;
      } else {
        actionBtn = `<button class="ob-btn-trade ob-btn-cancel" onclick="cancelOrder('${o.id}', '${o.hash160}')" title="Cancel your order">Cancel</button>`;
      }
    } else {
      if (o.isMatched) {
        actionBtn = `<span class="ob-status-capsule" style="background: rgba(245, 158, 11, 0.2); color: #f59e0b; border: 1px solid #f59e0b; padding: 2px 6px; border-radius: 4px; font-size: 0.7rem; font-weight: 700;">MATCHED</span>`;
      } else {
        actionBtn = `<button class="ob-btn-trade ob-btn-buy" onclick="takeOrder('${o.hash160}')">Buy</button>`;
      }
    }

    return `
      <tr class="ob-row ob-row-ask" style="background: linear-gradient(to left, rgba(244, 63, 94, 0.16) ${depthPct}%, transparent ${depthPct}%);" onclick="fillTradeForm(${o.price}, ${o.truAmount}, 'buy')" title="Click to fill Trade Form (Buy)">
        <td class="ob-cell-price ob-price-ask" style="padding-left: 12px;">
          <span class="price-val">$${o.price.toFixed(4)}</span>
        </td>
        <td class="ob-cell-amount" style="text-align: right;">${o.truAmount.toFixed(4)}</td>
        <td class="ob-cell-total" style="text-align: right;">$${o.total.toFixed(2)}</td>
        <td class="ob-cell-action" style="text-align: center;" onclick="event.stopPropagation()">
          ${actionBtn}
        </td>
      </tr>
    `;
  };

  const renderBidRow = (o) => {
    const depthPct = Math.min(100, Math.round((o.cumAmount / totalBidVol) * 100));
    const isMine = isMyOrder(o);
    let actionBtn = "";
    if (isMine) {
      if (o.isMatched) {
        actionBtn = `<span class="ob-status-capsule" style="background: rgba(245, 158, 11, 0.2); color: #f59e0b; border: 1px solid #f59e0b; padding: 2px 6px; border-radius: 4px; font-size: 0.7rem; font-weight: 700;">IN PROGRESS</span>`;
      } else {
        actionBtn = `<button class="ob-btn-trade ob-btn-cancel" onclick="cancelOrder('${o.id}', '${o.hash160}')" title="Cancel your order">Cancel</button>`;
      }
    } else {
      if (o.isMatched) {
        actionBtn = `<span class="ob-status-capsule" style="background: rgba(245, 158, 11, 0.2); color: #f59e0b; border: 1px solid #f59e0b; padding: 2px 6px; border-radius: 4px; font-size: 0.7rem; font-weight: 700;">MATCHED</span>`;
      } else {
        actionBtn = `<button class="ob-btn-trade ob-btn-sell" onclick="takeOrder('${o.hash160}')">Sell</button>`;
      }
    }

    return `
      <tr class="ob-row ob-row-bid" style="background: linear-gradient(to left, rgba(0, 230, 118, 0.16) ${depthPct}%, transparent ${depthPct}%);" onclick="fillTradeForm(${o.price}, ${o.truAmount}, 'sell')" title="Click to fill Trade Form (Sell)">
        <td class="ob-cell-price ob-price-bid" style="padding-left: 12px;">
          <span class="price-val">$${o.price.toFixed(4)}</span>
        </td>
        <td class="ob-cell-amount" style="text-align: right;">${o.truAmount.toFixed(4)}</td>
        <td class="ob-cell-total" style="text-align: right;">$${o.total.toFixed(2)}</td>
        <td class="ob-cell-action" style="text-align: center;" onclick="event.stopPropagation()">
          ${actionBtn}
        </td>
      </tr>
    `;
  };

  if (currentFilter === 'asks') {
    if (asksAsc.length === 0) {
      tbody.innerHTML = `<tr><td colspan="4" style="text-align: center; color: var(--text-dim); padding: 25px; font-size: 0.85rem;">—</td></tr>`;
      return;
    }
    tbody.innerHTML = asksAsc.map(renderAskRow).join('');
    return;
  }

  if (currentFilter === 'bids') {
    if (bidsDesc.length === 0) {
      tbody.innerHTML = `<tr><td colspan="4" style="text-align: center; color: var(--text-dim); padding: 25px; font-size: 0.85rem;">—</td></tr>`;
      return;
    }
    tbody.innerHTML = bidsDesc.map(renderBidRow).join('');
    return;
  }

  // Dual Orderbook (book) - Pure CEX Style (like Binance / TradingView):
  // 1. Asks at top: sorted descending (highest price at top, lowest/best ask at bottom touching the spread)
  // 2. Center Spread line: Last price and current spread
  // 3. Bids at bottom: sorted descending (highest/best bid at top touching the spread, lowest at bottom)
  let html = '';

  const asksForDual = [...asksAsc].sort((a, b) => b.price - a.price);
  if (asksForDual.length > 0) {
    html += asksForDual.map(renderAskRow).join('');
  } else {
    html += `<tr class="ob-empty-row"><td colspan="4" style="text-align: center; color: var(--text-dim); padding: 12px; font-size: 0.75rem;">—</td></tr>`;
  }

  // 2. Spread & Market Price Divider
  html += `
    <tr class="ob-spread-row">
      <td colspan="4">
        <div class="ob-spread-badge">
          <div class="ob-spread-price">
            ${lastPriceStr} <span class="ob-spread-unit">USDC</span>
          </div>
          <div class="ob-spread-info">
            Spread: <strong style="color: #cbd5e1;">${spreadStr}</strong>
          </div>
        </div>
      </td>
    </tr>
  `;

  // 3. Bids (Buys)
  if (bidsDesc.length > 0) {
    html += bidsDesc.map(renderBidRow).join('');
  } else {
    html += `<tr class="ob-empty-row"><td colspan="4" style="text-align: center; color: var(--text-dim); padding: 12px; font-size: 0.75rem;">—</td></tr>`;
  }

  tbody.innerHTML = html;
}

// Render Trade History of on-chain swaps in responsive vertical card interface
let currentHistoryLimit = 30;

function loadMoreHistory() {
  currentHistoryLimit += 30;
  renderTradeHistory();
}
window.loadMoreHistory = loadMoreHistory;

async function renderTradeHistory() {
  const historyContainer = document.getElementById('trade-history-container');
  const tableContainer = document.querySelector('.table-container.orderbook-frame');

  const trackerEl = document.getElementById('active-swaps-tracker');

  if (currentFilter === 'history' || currentFilter === 'ledger') {
    if (tableContainer) tableContainer.style.display = 'none';
    if (trackerEl) trackerEl.style.display = 'none';
    if (historyContainer) historyContainer.style.display = 'flex';
  }

  if (!historyContainer) return;

  if (!historicalLedgerData && ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ action: "req_swaps_ledger" }));
  }

  const swaps = (historicalLedgerData && historicalLedgerData.swaps) ? historicalLedgerData.swaps : [];

  if (swaps.length === 0) {
    historyContainer.innerHTML = `
      <div class="hist-empty-card">
        <div class="hist-empty-icon">📜</div>
        <div class="hist-empty-title">No Historical Swaps</div>
        <div class="hist-empty-desc">No atomic swaps found in scanned blocks yet. Completed trades and on-chain HTLC locks will appear here in real time.</div>
      </div>
    `;
    return;
  }

  const settledHashes = new Set();
  for (const s of swaps) {
    const cleanH = (s.hash160_hex || '').replace(/^0x/, '').toLowerCase();
    if (s.status === 'Claimed' || s.status === 'Refunded' || s.is_spent || s.spent_by_txid || s.base_claimed || s.vault_status === 1 || s.vault_status === 2) {
      settledHashes.add(cleanH);
    }
  }

  const activeSwaps = swaps.filter(s => (s.status === 'Active' || s.status === 'ExpiringSoon') && !s.is_spent && !s.base_claimed && !settledHashes.has((s.hash160_hex || '').replace(/^0x/, '').toLowerCase()));
  const settledSwaps = swaps.filter(s => (s.status !== 'Active' && s.status !== 'ExpiringSoon') || s.is_spent || s.base_claimed || settledHashes.has((s.hash160_hex || '').replace(/^0x/, '').toLowerCase()));

  activeSwaps.sort((a, b) => b.block_height - a.block_height);
  settledSwaps.sort((a, b) => b.block_height - a.block_height);

  const renderActiveCard = (s) => {
    const isBuy = s.direction === 2;
    const sideClass = isBuy ? 'side-buy' : 'side-sell';
    const sideText = isBuy ? '🟢 BUY TRU' : '🔴 SELL TRU';
    const timeDisplay = s.hours_remaining !== undefined
      ? `~${s.hours_remaining.toFixed(1)}h left`
      : 'Active';
    const truAmt = Number(s.amount_tru) || 0;
    const usdcAmt = Number(s.amount_usdc) || truAmt;
    const price = truAmt > 0 ? (usdcAmt / truAmt) : 1.0;
    const cleanHash = (s.hash160_hex || '').replace(/^0x/, '').toLowerCase();
    const shortHash = cleanHash.length >= 10 ? `${cleanHash.slice(0, 4)}...${cleanHash.slice(-4)}` : cleanHash;
    const targetSwapId = s.base_swap_id || '';
    const confs = Number(s.confirmations) || 0;
    const isSafe = s.is_safe || confs >= 6;

    let actionHtml = '';
    const hasBaseSwap = targetSwapId && targetSwapId.startsWith('0x') && targetSwapId.length === 66 && !targetSwapId.includes(':');

    if (isBuy) {
      // BUY TRU ORDER (Maker is buying TRU with USDC)
      if (s.is_mine || myCreatedOrders.has(cleanHash)) {
        if (!hasBaseSwap) {
          if (confs >= 3) {
            const takerBase = s.taker_base || '';
            actionHtml = `<button class="hist-action-btn hist-btn-blue" onclick="depositBaseHtlcForBuy('${cleanHash}', ${usdcAmt}, '${takerBase}', '${s.txid}', ${s.vout}, ${truAmt})" title="Seller TRU confirmed (${confs}/6 blocks)! Click to deposit ${usdcAmt.toFixed(2)} USDC on ${BASE_NETWORK_NAME} to complete swap">🔒 Lock ${usdcAmt.toFixed(2)} USDC on Base</button>`;
          } else {
            actionHtml = `<div class="hist-status-note note-warning" title="Seller TRU must reach at least 3 confirmations before manual lock">⏳ Awaiting 3 TRU Confs (${confs}/3)...</div>`;
          }
        } else if (isSafe || confs >= 6) {
          actionHtml = `<button class="hist-action-btn hist-btn-green" onclick="claimTruHtlc('${cleanHash}', '${s.txid}', ${s.vout}, ${truAmt})" title="USDC locked on Base! Click to claim ${truAmt.toFixed(4)} TRU to your local vault">⚡ Claim ${truAmt.toFixed(4)} TRU</button>`;
        } else {
          actionHtml = `<button class="hist-action-btn hist-btn-disabled" disabled title="Awaiting 6 TRU confirmations before claiming">⏳ Awaiting ${Math.max(1, 6 - confs)} blk (${confs}/6)...</button>`;
        }
      } else {
        // Taker
        const hasRealSecret = Boolean(s.has_secret && s.secret && s.secret !== '0000000000000000000000000000000000000000000000000000000000000000');
        if (!hasBaseSwap) {
          actionHtml = `<div class="hist-status-note note-warning" title="You locked TRU. Waiting for Buyer to deposit USDC on ${BASE_NETWORK_NAME}">⏳ Awaiting Buyer USDC...</div>`;
        } else if (hasRealSecret) {
          actionHtml = `<button class="hist-action-btn hist-btn-green" onclick="claimBaseHtlc('${targetSwapId}', '${cleanHash}')" title="Click to claim ${usdcAmt.toFixed(2)} USDC on ${BASE_NETWORK_NAME}">⚡ Claim ${usdcAmt.toFixed(2)} USDC ↗</button>`;
        } else {
          actionHtml = `<div class="hist-status-note note-warning" title="Waiting for Buyer to claim TRU on-chain to reveal preimage secret">⏳ Waiting for Buyer TRU Claim...</div>`;
        }
      }
    } else {
      // SELL TRU ORDER
      if (s.is_mine || myCreatedOrders.has(cleanHash)) {
        if (!isSafe) {
          actionHtml = `<div class="hist-status-note note-warning" title="Awaiting 6 TRU block confirmations before counterparty locks USDC on Base">⏳ Awaiting 6 TRU Confs (${confs}/6)</div>`;
        } else if (hasBaseSwap) {
          actionHtml = `<button class="hist-action-btn hist-btn-green" onclick="claimBaseHtlc('${targetSwapId}', '${cleanHash}')" title="Click to claim ${usdcAmt.toFixed(2)} USDC on ${BASE_NETWORK_NAME} using secret from your vault!">⚡ Claim ${usdcAmt.toFixed(2)} USDC ↗</button>`;
        } else {
          actionHtml = `<div class="hist-status-note note-info" title="TRU confirmed! Waiting for counterparty to deposit USDC in BaseHTLC">⏳ Awaiting USDC Deposit</div>`;
        }
      } else {
        actionHtml = `<a href="${BASE_EXPLORER_URL}/address/${BASE_HTLC_ADDR}" target="_blank" class="hist-badge hist-badge-base" style="font-weight: 600;" title="HTLC Locked on-chain">HTLC Locked ↗</a>`;
      }
    }

    const baseLockTx = s.base_lock_tx || '';
    const truLockTx = s.txid || '';
    const shortBaseTx = baseLockTx ? (baseLockTx.startsWith('0x') ? `0x${baseLockTx.slice(2, 6)}...${baseLockTx.slice(-4)}` : `${baseLockTx.slice(0, 4)}...${baseLockTx.slice(-4)}`) : '';
    const shortTruTx = truLockTx ? `${truLockTx.slice(0, 6)}...${truLockTx.slice(-4)}` : '';

    return `
      <div class="history-deal-card card-active">
        <!-- 1. Header -->
        <div class="hist-card-header">
          <div class="hist-card-badges">
            <span class="hist-badge-side ${sideClass}">${sideText}</span>
            <span class="hist-badge-status status-active">⚡ ACTIVE LOCK</span>
          </div>
          <div class="hist-card-meta">
            <span class="hist-block-pill" onclick="if(typeof openBlockModal==='function')openBlockModal(${s.block_height}, event)" title="View Block #${s.block_height}">#${s.block_height}</span>
            <span class="hist-time-pill">${timeDisplay}</span>
          </div>
        </div>

        <!-- 2. Amounts Box -->
        <div class="hist-card-amounts-box">
          <div class="hist-amounts-grid">
            <div class="hist-amt-col">
              <span class="hist-amt-label">TRU Amount</span>
              <span class="hist-amt-value hist-tru-val">${truAmt.toFixed(4)} <span class="hist-unit">TRU</span></span>
            </div>
            <div class="hist-swap-arrow">⇄</div>
            <div class="hist-amt-col">
              <span class="hist-amt-label">USDC Total</span>
              <span class="hist-amt-value hist-usdc-val">${usdcAmt.toFixed(2)} <span class="hist-unit">USDC</span></span>
            </div>
          </div>
          <div class="hist-rate-row">
            <span class="hist-rate-label">Rate:</span>
            <span class="hist-rate-val">$${price.toFixed(4)} / TRU</span>
          </div>
        </div>

        <!-- 3. Vertical Records & TX Proofs -->
        <div class="hist-card-details">
          ${truLockTx ? `
            <div class="hist-detail-row">
              <span class="hist-detail-label">TRU Lock:</span>
              <div class="hist-detail-val">
                <span class="hist-badge hist-badge-tru copyable-tx" onclick="openTruTxModal('${truLockTx}', event)" title="View in TRU Explorer">
                  <span class="hist-tx-mono">${shortTruTx}</span> 🔍
                </span>
              </div>
            </div>
          ` : ''}

          ${baseLockTx ? `
            <div class="hist-detail-row">
              <span class="hist-detail-label">Base Lock:</span>
              <div class="hist-detail-val">
                <a href="${BASE_EXPLORER_URL}/tx/${baseLockTx}" target="_blank" class="hist-badge hist-badge-base" onclick="event.stopPropagation()" title="View Base Lock on Basescan">
                  <span class="hist-tx-mono">${shortBaseTx}</span> ↗
                </a>
              </div>
            </div>
          ` : ''}

          <div class="hist-detail-row">
            <span class="hist-detail-label">Status:</span>
            <div class="hist-detail-val">
              <span class="hist-status-capsule-active">🔒 LOCKED ON-CHAIN</span>
            </div>
          </div>

          ${cleanHash ? `
            <div class="hist-detail-row">
              <span class="hist-detail-label">Swap Hash:</span>
              <div class="hist-detail-val">
                <span class="hist-badge hist-badge-hash copyable-tx" onclick="copyToClipboard('${cleanHash}', '✓ Swap Hash copied!', event)" title="Click to copy full secret hash">
                  <span class="hist-tx-mono">${shortHash}</span> 📋
                </span>
              </div>
            </div>
          ` : ''}
        </div>

        <!-- 4. Actions -->
        ${actionHtml ? `
          <div class="hist-card-actions">
            ${actionHtml}
          </div>
        ` : ''}
      </div>
    `;
  };

  const renderSettledCard = (s) => {
    const isBuy = s.direction === 2;
    const sideClass = isBuy ? 'side-buy' : 'side-sell';
    const sideText = isBuy ? '🟢 BUY TRU' : '🔴 SELL TRU';
    const timeDisplay = s.timelock
      ? new Date(s.timelock * 1000).toLocaleString([], { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })
      : (s.block_height ? `Block #${s.block_height}` : '—');
    const truAmt = Number(s.amount_tru) || 0;
    const usdcAmt = Number(s.amount_usdc) || (truAmt > 0 ? truAmt * 4.40 : 4.40);
    const price = truAmt > 0 ? (usdcAmt / truAmt) : 1.0;
    const shortClaimTx = s.spent_by_txid ? `${s.spent_by_txid.slice(0, 6)}...${s.spent_by_txid.slice(-4)}` : 'On-Chain';
    const isRefunded = s.status === 'Refunded';

    const cleanHash = (s.hash160_hex || '').replace(/^0x/, '').toLowerCase();
    const shortHash = cleanHash.length >= 10 ? `${cleanHash.slice(0, 4)}...${cleanHash.slice(-4)}` : cleanHash;

    let targetSwapId = (s.base_swap_id || '').trim();
    if (targetSwapId.includes(':') || targetSwapId.length < 64) {
      targetSwapId = '';
    }
    const cleanTargetId = targetSwapId ? targetSwapId.toLowerCase().replace(/^0x/, '') : '';

    const isBaseClaimed = Boolean(
      s.base_claimed ||
      (typeof claimedBaseSwaps !== 'undefined' && (
        claimedBaseSwaps.has(cleanHash) ||
        (cleanTargetId && (claimedBaseSwaps.has(cleanTargetId) || claimedBaseSwaps.has('0x' + cleanTargetId)))
      ))
    );

    let statusBadgeHtml = '';
    let cardModifierClass = '';
    if (isRefunded) {
      statusBadgeHtml = `<span class="hist-badge-status status-refunded">↩ REFUNDED</span>`;
      cardModifierClass = 'card-refunded';
    } else if (isBaseClaimed) {
      statusBadgeHtml = `<span class="hist-badge-status status-claimed">✓ BASE CLAIMED</span>`;
      cardModifierClass = 'card-claimed';
    } else {
      statusBadgeHtml = `<span class="hist-badge-status status-claimed">✓ TRU CLAIMED</span>`;
      cardModifierClass = 'card-claimed';
    }

    let actionBtnHtml = '';
    if (!isRefunded && !isBaseClaimed && cleanTargetId && cleanTargetId.length >= 64) {
      const fullSwapId = '0x' + cleanTargetId;
      actionBtnHtml = `<button class="hist-action-btn hist-btn-green" onclick="claimBaseHtlc('${fullSwapId}', '${cleanHash}')" title="TRU claimed on-chain! Click to claim ${usdcAmt.toFixed(2)} USDC on ${BASE_NETWORK_NAME}">⚡ Claim ${usdcAmt.toFixed(2)} USDC ↗</button>`;
      if (typeof checkBaseSwapClaimed === 'function') {
        checkBaseSwapClaimed(fullSwapId, cleanHash);
      }
    }

    const baseLockTx = s.base_lock_tx || '';
    const truLockTx = s.txid || '';
    const shortBaseTx = baseLockTx ? (baseLockTx.startsWith('0x') ? `0x${baseLockTx.slice(2, 6)}...${baseLockTx.slice(-4)}` : `${baseLockTx.slice(0, 4)}...${baseLockTx.slice(-4)}`) : '';
    const shortTruTx = truLockTx ? `${truLockTx.slice(0, 6)}...${truLockTx.slice(-4)}` : '';

    return `
      <div class="history-deal-card ${cardModifierClass}">
        <!-- 1. Header -->
        <div class="hist-card-header">
          <div class="hist-card-badges">
            <span class="hist-badge-side ${sideClass}">${sideText}</span>
            ${statusBadgeHtml}
          </div>
          <div class="hist-card-meta">
            <span class="hist-block-pill" onclick="if(typeof openBlockModal==='function')openBlockModal(${s.block_height}, event)" title="View Block #${s.block_height}">#${s.block_height}</span>
            <span class="hist-time-pill">${timeDisplay}</span>
          </div>
        </div>

        <!-- 2. Amounts Box -->
        <div class="hist-card-amounts-box">
          <div class="hist-amounts-grid">
            <div class="hist-amt-col">
              <span class="hist-amt-label">TRU Amount</span>
              <span class="hist-amt-value hist-tru-val">${truAmt.toFixed(4)} <span class="hist-unit">TRU</span></span>
            </div>
            <div class="hist-swap-arrow">⇄</div>
            <div class="hist-amt-col">
              <span class="hist-amt-label">USDC Total</span>
              <span class="hist-amt-value hist-usdc-val">${usdcAmt.toFixed(2)} <span class="hist-unit">USDC</span></span>
            </div>
          </div>
          <div class="hist-rate-row">
            <span class="hist-rate-label">Effective Rate:</span>
            <span class="hist-rate-val">$${price.toFixed(4)} / TRU</span>
          </div>
        </div>

        <!-- 3. Vertical Records & TX Proofs -->
        <div class="hist-card-details">
          ${truLockTx ? `
            <div class="hist-detail-row">
              <span class="hist-detail-label">TRU Lock:</span>
              <div class="hist-detail-val">
                <span class="hist-badge hist-badge-tru copyable-tx" onclick="openTruTxModal('${truLockTx}', event)" title="View in TRU Explorer">
                  <span class="hist-tx-mono">${shortTruTx}</span> 🔍
                </span>
              </div>
            </div>
          ` : ''}

          ${baseLockTx ? `
            <div class="hist-detail-row">
              <span class="hist-detail-label">Base Lock:</span>
              <div class="hist-detail-val">
                <a href="${BASE_EXPLORER_URL}/tx/${baseLockTx}" target="_blank" class="hist-badge hist-badge-base" onclick="event.stopPropagation()" title="View on Basescan">
                  <span class="hist-tx-mono">${shortBaseTx}</span> ↗
                </a>
              </div>
            </div>
          ` : ''}

          <div class="hist-detail-row">
            <span class="hist-detail-label">Settlement:</span>
            <div class="hist-detail-val">
              ${s.spent_by_txid ? `
                <span class="hist-badge hist-badge-claim copyable-tx" onclick="openTruTxModal('${s.spent_by_txid}', event)" title="View Claim Tx in TRU Explorer">
                  <span class="hist-tx-mono">${shortClaimTx}</span> 🔍
                </span>
              ` : (isBaseClaimed ? `
                <a href="${BASE_EXPLORER_URL}/address/${BASE_HTLC_ADDR}" target="_blank" class="hist-badge hist-badge-claim" onclick="event.stopPropagation()" title="USDC Claimed on Base">
                  <span class="hist-tx-mono">Base HTLC</span> ↗
                </a>
              ` : `
                <span class="hist-status-note note-info">On-Chain</span>
              `)}
            </div>
          </div>

          ${cleanHash ? `
            <div class="hist-detail-row">
              <span class="hist-detail-label">Swap Hash:</span>
              <div class="hist-detail-val">
                <span class="hist-badge hist-badge-hash copyable-tx" onclick="copyToClipboard('${cleanHash}', '✓ Swap Hash copied!', event)" title="Click to copy full secret hash">
                  <span class="hist-tx-mono">${shortHash}</span> 📋
                </span>
              </div>
            </div>
          ` : ''}
        </div>

        <!-- 4. Actions -->
        ${actionBtnHtml ? `
          <div class="hist-card-actions">
            ${actionBtnHtml}
          </div>
        ` : ''}
      </div>
    `;
  };

  const visibleSettledSwaps = settledSwaps.slice(0, currentHistoryLimit);
  const hasMore = settledSwaps.length > currentHistoryLimit;

  let loadMoreHtml = '';
  if (hasMore) {
    const remaining = settledSwaps.length - currentHistoryLimit;
    const nextBatch = Math.min(30, remaining);
    loadMoreHtml = `
      <div class="hist-load-more-container">
        <button type="button" class="hist-load-more-btn" onclick="loadMoreHistory()">
          <span>⬇ Load More Swaps (+${nextBatch})</span>
          <span class="hist-load-more-badge">· Showing ${visibleSettledSwaps.length + activeSwaps.length} of ${swaps.length}</span>
        </button>
      </div>
    `;
  }

  const headerSummaryHtml = `
    <div class="hist-panel-summary">
      <div class="hist-summary-left">
        <span class="hist-summary-icon">📜</span>
        <span class="hist-summary-title">Atomic Swaps Ledger</span>
        <span class="hist-summary-count">${visibleSettledSwaps.length + activeSwaps.length} / ${swaps.length} total</span>
      </div>
      <div class="hist-summary-right">
        ${activeSwaps.length > 0 ? `<span class="hist-pill-active">${activeSwaps.length} Active</span>` : ''}
        <span class="hist-pill-settled">${settledSwaps.length} Settled</span>
      </div>
    </div>
  `;

  historyContainer.innerHTML = headerSummaryHtml + activeSwaps.map(renderActiveCard).join('') + visibleSettledSwaps.map(renderSettledCard).join('') + loadMoreHtml;
}

// --- In-Progress Swaps & ConfirmationGuard Tracker ---
const pendingTakenSwaps = new Map();

function registerPendingTakeSwap(order) {
  if (!order) return;
  const cleanHash = (order.hash160 || order.hash160_hex || '').replace(/^0x/, '').toLowerCase();
  if (!cleanHash) return;
  pendingTakenSwaps.set(cleanHash, {
    hash160_hex: cleanHash,
    direction: (order.side === 'SELL_TRU' || order.side === 'SellTruForBaseUsdc' || order.side === 2) ? 2 : 1,
    amount_tru: Number(order.truAmount) || 0,
    amount_usdc: Number(order.usdcAmount) || Number(order.truAmount) || 0,
    confirmations: 0,
    is_safe: false,
    status: 'Active',
    hours_remaining: 4.0,
    timelock: order.timelock || Math.floor(Date.now() / 1000) + 14400,
    txid: (order.txid && order.txid !== order.id) ? order.txid : '',
    is_mine: true,
    timestamp: Date.now()
  });

  renderActiveSwapsTracker();

  // Highlight tracker panel and scroll smoothly
  const trackerEl = document.getElementById('active-swaps-tracker');
  if (trackerEl) {
    trackerEl.style.display = 'block';
    trackerEl.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    const card = trackerEl.querySelector('.in-progress-deal-card');
    if (card) {
      card.classList.add('deal-highlight');
      setTimeout(() => card.classList.remove('deal-highlight'), 2000);
    }
  }
}
window.registerPendingTakeSwap = registerPendingTakeSwap;

function getActiveInProgressSwaps() {
  const swaps = (historicalLedgerData && historicalLedgerData.swaps) ? historicalLedgerData.swaps : [];
  const list = [];
  const seenHashes = new Set();

  const settledHashes = new Set();
  for (const s of swaps) {
    const cleanH = (s.hash160_hex || '').replace(/^0x/, '').toLowerCase();
    if (s.status === 'Claimed' || s.status === 'Refunded' || s.is_spent || s.spent_by_txid || s.base_claimed || s.vault_status === 1 || s.vault_status === 2) {
      settledHashes.add(cleanH);
    }
  }

  for (const s of swaps) {
    const cleanH = (s.hash160_hex || '').replace(/^0x/, '').toLowerCase();
    if ((s.status !== 'Active' && s.status !== 'ExpiringSoon') || s.is_spent || s.spent_by_txid || s.base_claimed) continue;
    if (settledHashes.has(cleanH)) continue;
    if (s.vault_status === 1 || s.vault_status === 2) continue;
    // An open limit order in the orderbook (unmatched, 0 funds locked on TRU or Base) is NOT an in-progress swap!
    // But if on-chain locks exist (s.txid on TRU or s.base_swap_id on Base), it is actively in-progress!
    if (!s.txid && !s.base_swap_id && (s.vault_step === 1 || !s.is_matched)) continue;

    const isMine = s.is_mine || (typeof myCreatedOrders !== 'undefined' && myCreatedOrders.has(cleanH)) || pendingTakenSwaps.has(cleanH);
    if (!isMine) continue;

    if (!seenHashes.has(cleanH)) {
      seenHashes.add(cleanH);
      list.push(s);
    }
  }

  const now = Date.now();
  for (const [hash, pending] of pendingTakenSwaps.entries()) {
    if (now - pending.timestamp > 1800000 || settledHashes.has(hash)) {
      pendingTakenSwaps.delete(hash);
      continue;
    }
    if (!seenHashes.has(hash)) {
      list.push(pending);
    } else {
      pendingTakenSwaps.delete(hash);
    }
  }

  return list;
}

function renderActiveSwapsTracker() {
  const trackerEl = document.getElementById('active-swaps-tracker');
  const badgeEl = document.getElementById('tab-inprogress-badge');
  const activeDeals = getActiveInProgressSwaps();

  if (badgeEl) {
    badgeEl.innerText = activeDeals.length;
  }

  if (!trackerEl) return;

  if (activeDeals.length === 0 || currentFilter === 'history' || currentFilter === 'ledger') {
    trackerEl.style.display = 'none';
    if (activeDeals.length === 0) trackerEl.innerHTML = '';
    return;
  }

  trackerEl.style.display = 'block';

  let html = `
    <div class="tracker-panel-header">
      <div class="tracker-panel-title-group">
        <span class="tracker-pulse-dot"></span>
        <span class="tracker-panel-title">Active Swaps in Progress</span>
        <span class="tracker-panel-count-badge">${activeDeals.length} ACTIVE</span>
      </div>
      <div class="tracker-guard-capsule">
        <span>🛡️ Guard:</span> <strong>≥ 6 TRU Blocks</strong>
      </div>
    </div>
  `;

  for (const s of activeDeals) {
    const isBuy = s.direction === 2;
    const sideClass = isBuy ? 'side-buy' : 'side-sell';
    const sideText = isBuy ? '🟢 BUY TRU' : '🔴 SELL TRU';
    const truAmt = Number(s.amount_tru) || 0;
    const usdcAmt = Number(s.amount_usdc) || truAmt;
    const rate = truAmt > 0 ? (usdcAmt / truAmt).toFixed(4) : '1.0000';
    const confs = Number(s.confirmations) || 0;
    const isSafe = s.is_safe || confs >= 6;
    const percent = Math.min(100, Math.round((Math.min(confs, 6) / 6) * 100));
    const cleanHash = (s.hash160_hex || '').replace(/^0x/, '').toLowerCase();
    const shortHash = cleanHash.length >= 10 ? `${cleanHash.slice(0, 4)}...${cleanHash.slice(-4)}` : cleanHash;
    const targetSwapId = s.base_swap_id || '';
    const hasBaseSwap = targetSwapId && targetSwapId.startsWith('0x') && targetSwapId.length === 66;

    const timeDisplay = s.hours_remaining !== undefined && s.hours_remaining > 0
      ? `⏳ ~${s.hours_remaining.toFixed(1)}h left`
      : 'Active';

    const baseLockTx = s.base_lock_tx || '';
    const truLockTx = s.txid || '';

    const isTruMined = Boolean((s.block_height && s.block_height > 0) || confs > 0);
    const isTruBroadcasted = Boolean(truLockTx);

    const isSettled = Boolean(s.status === 'Claimed' || s.is_spent || s.base_claimed);
    const barClass = (isSafe || isSettled) ? 'bar-safe' : 'bar-confirming';
    const barWidth = isSettled ? 100 : Math.max(8, percent);

    const statusHeadline = isSettled
      ? `🎉 Swap Settled`
      : (isSafe
        ? `✅ TRU Confirmed: 6/6`
        : (isTruMined
          ? `⏳ TRU: ${confs}/6 Confs`
          : (isTruBroadcasted
            ? `⏳ TRU in Mempool`
            : `⏳ Waiting Lock...`)));

    const statusSubtext = isSettled
      ? `Claimed`
      : (isSafe
        ? `Settling...`
        : (isTruMined
          ? `${Math.max(0, 6 - confs)} left`
          : (isTruBroadcasted
            ? `Mining (0/6)`
            : `Pending`)));

    const step1Done = Boolean(hasBaseSwap && (baseLockTx || s.base_claimed || s.status === 'Claimed'));
    const step2Done = isTruMined;

    let actionBtnHtml = '';
    if (isSettled) {
      actionBtnHtml = `<span class="ob-status-capsule status-claimed" style="padding: 2px 6px; font-weight: 700; font-size: 0.69rem;">✓ SETTLED</span>`;
    } else if (isSafe && hasBaseSwap) {
      actionBtnHtml = `<span style="color: #38bdf8; font-weight: 700; font-size: 0.70rem; font-family: var(--font-mono); display: inline-flex; align-items: center; gap: 4px; white-space: nowrap;"><span class="tracker-automation-pulse"></span> ⚡ Settling</span>`;
    } else if (isSafe && !hasBaseSwap) {
      actionBtnHtml = `<span style="color: #60a5fa; font-weight: 600; font-size: 0.70rem; font-family: var(--font-mono); white-space: nowrap;">⏳ Wait USDC</span>`;
    } else if (confs > 0) {
      actionBtnHtml = `<span style="color: #fbbf24; font-weight: 700; font-size: 0.70rem; font-family: var(--font-mono); white-space: nowrap;">⏳ ${confs}/6 Confs</span>`;
    } else if (isTruBroadcasted) {
      actionBtnHtml = `<span style="color: #fbbf24; font-weight: 700; font-size: 0.70rem; font-family: var(--font-mono); display: inline-flex; align-items: center; gap: 4px; white-space: nowrap;">⛏️ Mining...</span>`;
    } else {
      actionBtnHtml = `<span style="color: #94a3b8; font-weight: 600; font-size: 0.70rem; font-family: var(--font-mono); white-space: nowrap;">🤖 Active</span>`;
    }

    const shortBaseTx = baseLockTx ? (baseLockTx.startsWith('0x') ? `0x${baseLockTx.slice(2, 6)}...${baseLockTx.slice(-4)}` : `${baseLockTx.slice(0, 4)}...${baseLockTx.slice(-4)}`) : '';
    const shortTruTx = truLockTx ? `${truLockTx.slice(0, 4)}...${truLockTx.slice(-4)}` : '';

    const step1TxHtml = baseLockTx
      ? `<a href="${BASE_EXPLORER_URL}/tx/${baseLockTx}" target="_blank" class="tracker-step-tx tracker-link-base" title="View Base Lock TX on Basescan" onclick="event.stopPropagation()">TX: ${shortBaseTx} ↗</a>`
      : '';

    const step2TxHtml = truLockTx
      ? `<span class="tracker-step-tx tracker-link-tru copyable-tx" style="cursor: pointer;" onclick="openTruTxModal('${truLockTx}', event)" title="View TRU Lock in Built-in Explorer: ${truLockTx}">TX: ${shortTruTx} 🔍</span>`
      : '';

    const hasSecret = Boolean(s.has_secret || (s.secret && !s.secret.startsWith('00000000') && s.secret.length === 64));
    const isBaseFirst = (isBuy === hasSecret) || (hasBaseSwap && !isSafe);

    const confBadge = isSafe
      ? `<span class="tracker-step-tx tracker-badge-safe">🛡️ Safe (≥6)</span>`
      : (confs > 0 ? `<span class="tracker-step-tx tracker-badge-mining">⛏️ ${confs}/6 Confs</span>` : `<span class="tracker-step-tx tracker-badge-pending">⏳ 0/6 Confs</span>`);

    let pipelineHtml = '';
    if (isBaseFirst) {
      // BUYER-INITIATED SWAP (Buyer locks Base USDC first): 1. Base USDC -> 2. TRU Lock -> 3. Confirmations -> 4. Settlement
      const p1Done = step1Done;
      const p1Class = isSettled || p1Done ? 'step-completed' : 'step-active';
      const p1Status = (isSettled || p1Done) ? '✔ Locked' : '⏳ Deposit...';
      const p1Badge = step1TxHtml || (p1Done ? `<span class="tracker-step-tx tracker-badge-safe">✔ Locked</span>` : `<span class="tracker-step-tx tracker-badge-pending">⏳ Pending</span>`);

      const p2Done = step2Done;
      const p2Class = isSettled || p2Done ? 'step-completed' : (isTruBroadcasted ? 'step-active' : 'step-pending');
      const p2Status = (isSettled || p2Done) ? (s.block_height ? `✔ #${s.block_height}` : '✔ Mined') : (isTruBroadcasted ? '⏳ In Mempool' : '⏳ Wait TRU');
      const p2Badge = step2TxHtml || (p2Done ? `<span class="tracker-step-tx tracker-badge-safe">✔ Mined</span>` : (isTruBroadcasted ? `<span class="tracker-step-tx tracker-badge-mining">⛏️ Mempool</span>` : `<span class="tracker-step-tx tracker-badge-pending">⏳ Pending</span>`));

      const p3Done = isSettled || isSafe;
      const p3Class = isSettled || p3Done ? 'step-completed' : (p2Done ? 'step-active' : 'step-pending');
      const p3Status = isSettled ? (confs >= 6 ? '✔ 6/6 Confirmed' : `✔ ${confs}/6 (OVERRIDE)`) : (isSafe ? `✔ 6/6 Confirmed` : (p2Done ? `⏳ ${confs}/6 blocks` : '⏳ Waiting'));
      const p3Badge = confBadge;

      const p4Done = isSettled;
      const p4Class = p4Done ? 'step-completed' : (p3Done ? 'step-active' : 'step-pending');
      const p4Status = p4Done ? '✔ Claimed' : (isSafe ? '⚡ Ready' : '⏳ Waiting');
      const p4Badge = p4Done
        ? `<span class="tracker-step-tx tracker-badge-safe">🎉 Settled</span>`
        : (isSafe ? `<span class="tracker-step-tx tracker-badge-ready">⚡ Ready</span>` : `<span class="tracker-step-tx tracker-badge-pending">⏳ Pending</span>`);

      pipelineHtml = `
        <div class="tracker-pipeline">
          <div class="tracker-pipe-step ${p1Class}">
            <div class="tracker-step-meta-top">
              <span class="tracker-step-title">1. Base USDC</span>
              <span class="tracker-step-status">${p1Status}</span>
            </div>
            ${p1Badge}
          </div>
          <div class="tracker-pipe-step ${p2Class}">
            <div class="tracker-step-meta-top">
              <span class="tracker-step-title">2. TRU Lock</span>
              <span class="tracker-step-status">${p2Status}</span>
            </div>
            ${p2Badge}
          </div>
          <div class="tracker-pipe-step ${p3Class}">
            <div class="tracker-step-meta-top">
              <span class="tracker-step-title">3. Confirmations</span>
              <span class="tracker-step-status">${p3Status}</span>
            </div>
            ${p3Badge}
          </div>
          <div class="tracker-pipe-step ${p4Class}">
            <div class="tracker-step-meta-top">
              <span class="tracker-step-title">4. Settlement</span>
              <span class="tracker-step-status">${p4Status}</span>
            </div>
            ${p4Badge}
          </div>
        </div>
      `;
    } else {
      // SELL TRU: 1. TRU Lock -> 2. Confirmations -> 3. Base USDC -> 4. Settlement
      const p1Done = step2Done;
      const p1Class = isSettled || p1Done ? 'step-completed' : (isTruBroadcasted ? 'step-active' : 'step-pending');
      const p1Status = (isSettled || p1Done) ? (s.block_height ? `✔ #${s.block_height}` : '✔ Mined') : (isTruBroadcasted ? '⏳ In Mempool' : '⏳ Pending');
      const p1Badge = step2TxHtml || (p1Done ? `<span class="tracker-step-tx tracker-badge-safe">✔ Mined</span>` : (isTruBroadcasted ? `<span class="tracker-step-tx tracker-badge-mining">⛏️ Mempool</span>` : `<span class="tracker-step-tx tracker-badge-pending">⏳ Pending</span>`));

      const p2Done = isSettled || isSafe;
      const p2Class = isSettled || p2Done ? 'step-completed' : (p1Done ? 'step-active' : 'step-pending');
      const p2Status = isSettled ? (confs >= 6 ? '✔ 6/6 Confirmed' : `✔ ${confs}/6 (OVERRIDE)`) : (isSafe ? `✔ 6/6 Confirmed` : (p1Done ? `⏳ ${confs}/6 blocks` : '⏳ Waiting'));
      const p2Badge = confBadge;

      const p3Done = hasBaseSwap;
      const p3Class = isSettled || p3Done ? 'step-completed' : (p2Done ? 'step-active' : 'step-pending');
      const p3Status = (isSettled || p3Done) ? '✔ Locked' : (p2Done ? '⏳ Wait USDC' : '⏳ Wait Confs');
      const p3Badge = step1TxHtml || (p3Done ? `<span class="tracker-step-tx tracker-badge-safe">✔ Locked</span>` : `<span class="tracker-step-tx tracker-badge-pending">⏳ Waiting</span>`);

      const p4Done = isSettled;
      const p4Class = p4Done ? 'step-completed' : (p3Done ? 'step-active' : 'step-pending');
      const p4Status = p4Done ? '✔ Claimed' : (p3Done ? '⚡ Ready' : '⏳ Waiting');
      const p4Badge = p4Done
        ? `<span class="tracker-step-tx tracker-badge-safe">🎉 Settled</span>`
        : (isSafe ? `<span class="tracker-step-tx tracker-badge-ready">⚡ Ready</span>` : `<span class="tracker-step-tx tracker-badge-pending">⏳ Pending</span>`);

      pipelineHtml = `
        <div class="tracker-pipeline">
          <div class="tracker-pipe-step ${p1Class}">
            <div class="tracker-step-meta-top">
              <span class="tracker-step-title">1. TRU Lock</span>
              <span class="tracker-step-status">${p1Status}</span>
            </div>
            ${p1Badge}
          </div>
          <div class="tracker-pipe-step ${p2Class}">
            <div class="tracker-step-meta-top">
              <span class="tracker-step-title">2. Confirmations</span>
              <span class="tracker-step-status">${p2Status}</span>
            </div>
            ${p2Badge}
          </div>
          <div class="tracker-pipe-step ${p3Class}">
            <div class="tracker-step-meta-top">
              <span class="tracker-step-title">3. Base USDC</span>
              <span class="tracker-step-status">${p3Status}</span>
            </div>
            ${p3Badge}
          </div>
          <div class="tracker-pipe-step ${p4Class}">
            <div class="tracker-step-meta-top">
              <span class="tracker-step-title">4. Settlement</span>
              <span class="tracker-step-status">${p4Status}</span>
            </div>
            ${p4Badge}
          </div>
        </div>
      `;
    }

    const baseFooterLink = baseLockTx
      ? `<a href="${BASE_EXPLORER_URL}/tx/${baseLockTx}" target="_blank" class="tracker-link tracker-link-base" title="Base USDC Lock TX: ${baseLockTx}">Base Lock: ${shortBaseTx} ↗</a>`
      : (targetSwapId ? `<a href="${BASE_EXPLORER_URL}/address/${BASE_HTLC_ADDR}" target="_blank" class="tracker-link" title="BaseHTLC Contract">BaseHTLC ↗</a>` : '');

    const truFooterLink = truLockTx
      ? `<span class="tracker-link tracker-link-tru copyable-tx" style="cursor: pointer;" onclick="openTruTxModal('${truLockTx}', event)" title="View TRU Lock in Built-in Explorer: ${truLockTx}">TRU Lock: ${shortTruTx} 🔍</span>`
      : '';

    html += `
      <div class="in-progress-deal-card" id="deal-${cleanHash}">
        <div class="tracker-deal-top">
          <div class="tracker-deal-header-row">
            <span class="tracker-deal-badge-side ${sideClass}">${sideText}</span>
            <div class="tracker-deal-timer">${timeDisplay}</div>
          </div>
          <div class="tracker-deal-amounts-row">
            <div class="tracker-deal-amounts">
              <span class="tracker-amt-item">${truAmt.toFixed(4)} <span class="tracker-token">TRU</span></span>
              <span class="tracker-amt-sep">↔</span>
              <span class="tracker-amt-item">${usdcAmt.toFixed(2)} <span class="tracker-token">USDC</span></span>
              <span class="tracker-deal-rate">(@ ${rate})</span>
            </div>
          </div>
        </div>

        <div class="tracker-progress-card">
          <div class="tracker-progress-labels">
            <span class="tracker-progress-headline">${statusHeadline}</span>
            <span class="tracker-progress-subtext">${statusSubtext}</span>
          </div>
          <div class="tracker-progress-track" title="Confirmations: ${confs} of 6">
            <div class="tracker-progress-fill ${barClass}" style="width: ${barWidth}%;"></div>
          </div>
        </div>

        ${pipelineHtml}

        <div class="tracker-automation-bar">
          <div class="tracker-automation-info">
            <span class="tracker-automation-pulse"></span>
            <span class="tracker-automation-tag">AUTO:</span>
            <span class="tracker-automation-preview" id="deal-log-preview-${cleanHash}">${escapeHtml(getLatestAutomationLogForSwap(s))}</span>
          </div>
          <button type="button" class="btn-open-audit-modal" onclick="openAutomationAuditModal('${cleanHash}')" title="Open full real-time automation process and audit log">
            📜 Audit
          </button>
        </div>

        <div class="tracker-deal-footer">
          <div class="tracker-footer-meta" title="Swap Secret Hash: ${cleanHash}">
            <span class="tracker-hash-label">Hash:</span>
            <span class="tracker-hash-val">${shortHash}</span>
            <button type="button" class="mecha-mini-btn" style="height: 18px; padding: 0 4px; font-size: 0.65rem;" onclick="copyTxToClipboard('${cleanHash}', event)" title="Copy Secret Hash">📋</button>
          </div>
          <div class="tracker-footer-action">
            ${actionBtnHtml}
          </div>
        </div>
      </div>
    `;
  }

  trackerEl.innerHTML = html;
}

function copyTxToClipboard(text, event) {
  if (event) {
    event.stopPropagation();
    event.preventDefault();
  }
  if (!text) return;
  const target = event ? (event.currentTarget || event.target) : null;
  const originalHtml = target ? target.innerHTML : '';

  const showSuccess = () => {
    if (target) {
      target.innerHTML = '✔ Copied!';
      target.style.color = '#10b981';
      setTimeout(() => {
        target.innerHTML = originalHtml;
        target.style.color = '';
      }, 1500);
    }
  };

  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(showSuccess).catch(() => {
      fallbackCopy(text);
      showSuccess();
    });
  } else {
    fallbackCopy(text);
    showSuccess();
  }
}

function fallbackCopy(text) {
  const ta = document.createElement('textarea');
  ta.value = text;
  ta.style.position = 'fixed';
  ta.style.opacity = '0';
  document.body.appendChild(ta);
  ta.select();
  try {
    document.execCommand('copy');
  } catch (e) {
    console.error('Copy failed:', e);
  }
  document.body.removeChild(ta);
}

function escapeHtml(str) {
  if (str === null || str === undefined) return '';
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#039;');
}
window.escapeHtml = escapeHtml;
window.copyTxToClipboard = copyTxToClipboard;

async function openTruTxModal(txid, event) {
  if (event) {
    if (typeof event.stopPropagation === 'function') event.stopPropagation();
    if (typeof event.preventDefault === 'function') event.preventDefault();
  }
  const cleanTxid = String(txid || '').trim();
  if (!cleanTxid) return;

  const targetUrl = '/explorer?tx=' + encodeURIComponent(cleanTxid);
  const win = window.open(targetUrl, '_blank');
  if (!win) {
    window.location.href = targetUrl;
  }
}

function closeTruTxModal() {
  const modal = document.getElementById('tru-tx-modal');
  if (!modal) return;
  modal.classList.remove('open');
  modal.style.display = 'none';
  document.body.classList.remove('modal-open');
}

function renderTruTxModalContent(data) {
  const body = document.getElementById('tru-tx-modal-body');
  if (!body) return;

  const isConfirmed = !!data.is_confirmed;
  const confs = data.confirmations !== undefined ? data.confirmations : (isConfirmed ? 1 : 0);
  const blockHeight = data.block_height !== null && data.block_height !== undefined ? data.block_height : 'Mempool Pending';
  const blockHash = data.block_hash || '—';
  const shortBlockHash = blockHash.length > 28 ? `${blockHash.slice(0, 14)}...${blockHash.slice(-10)}` : blockHash;

  const inputs = Array.isArray(data.inputs) ? data.inputs : [];
  const outputs = Array.isArray(data.outputs) ? data.outputs : [];

  const inputsHtml = inputs.length > 0 ? inputs.map((inp, idx) => {
    const isCb = inp.is_coinbase || (inp.prev_txid && (inp.prev_txid.toLowerCase().includes('genesis') || inp.prev_txid.split('').every(c => c === '0')));
    const prevTx = inp.prev_txid || '0000000000000000000000000000000000000000000000000000000000000000';
    const shortPrev = prevTx.length > 20 ? `${prevTx.slice(0, 9)}...${prevTx.slice(-7)}` : prevTx;
    return `
      <div class="tru-tx-io-item">
        <div class="tru-tx-target-group">
          ${isCb ? `<span style="color: #fbbf24; font-weight: 700;">🪙 Coinbase</span>` : `
            <span class="tru-tx-target-link" onclick="openTruTxModal('${escapeHtml(prevTx)}', event)" title="Inspect Prevout: ${escapeHtml(prevTx)}">${escapeHtml(shortPrev)}</span>
            <button type="button" class="mecha-mini-btn" style="height: 18px; padding: 0 4px; font-size: 0.65rem;" onclick="copyTxToClipboard('${escapeHtml(prevTx)}', event)" title="Copy Prevout TXID">📋</button>
          `}
        </div>
        <div style="color: #8c9ba5; font-size: 0.74rem; font-weight: 600;">
          ${isCb ? 'Newly Minted' : `Output #${inp.prev_vout !== undefined ? inp.prev_vout : idx}`}
        </div>
      </div>
    `;
  }).join('') : `
    <div class="tru-tx-io-item" style="justify-content: center; color: #64748b;">
      No inputs recorded
    </div>
  `;

  const outputsHtml = outputs.length > 0 ? outputs.map(out => {
    const target = out.display_target || out.address || `${data.txid}:${out.vout}`;
    const shortTarget = target.length > 20 ? `${target.slice(0, 10)}...${target.slice(-6)}` : target;
    const amt = typeof out.amount_tru === 'number' ? out.amount_tru : parseFloat(out.amount_tru || 0);
    const amtStr = amt.toFixed(amt > 0 && amt < 0.0001 ? 8 : (amt % 1 === 0 ? 0 : 7));

    return `
      <div class="tru-tx-io-item">
        <div class="tru-tx-target-group">
          <span class="tru-tx-target-text" title="${escapeHtml(target)}">${escapeHtml(shortTarget)}</span>
          ${out.address ? `<button type="button" class="mecha-mini-btn" style="height: 18px; padding: 0 4px; font-size: 0.65rem;" onclick="copyTxToClipboard('${escapeHtml(out.address)}', event)" title="Copy Address">📋</button>` : ''}
          ${out.is_htlc ? '<span class="mini-tag-htlc">HTLC</span>' : ''}
          ${out.is_vault ? '<span class="mini-tag-vault">Vault</span>' : ''}
        </div>
        <div class="tru-tx-io-amount">
          ${amtStr}<span class="tru-unit">TRU</span>
        </div>
      </div>
    `;
  }).join('') : `
    <div class="tru-tx-io-item" style="justify-content: center; color: #64748b;">
      No outputs
    </div>
  `;

  body.innerHTML = `
    <!-- Top Banner Card -->
    <div class="tru-tx-banner-box">
      <div style="display: flex; justify-content: space-between; align-items: center;">
        <span class="tru-tx-banner-label">Transaction Hash (TRU P2P)</span>
        <button type="button" class="mecha-mini-btn" onclick="copyTxToClipboard('${escapeHtml(data.txid)}', event)" title="Copy Full TXID">
          📋 Copy TXID
        </button>
      </div>
      <div class="tru-tx-hash-row">
        <span class="tru-tx-hash-val" id="tru-modal-txid" title="${escapeHtml(data.txid)}">${escapeHtml(data.txid)}</span>
      </div>
      <div class="tru-tx-badges-row">
        <span class="badge-confs" style="${isConfirmed ? '' : 'background: rgba(255, 184, 0, 0.12); color: var(--tru-gold); border-color: rgba(255, 184, 0, 0.3);'}">
          ${isConfirmed ? '● CONFIRMED' : '⏳ MEMPOOL'}
        </span>
        <span class="take-modal-pill-badge" style="color: #60a5fa; font-weight: 600;">
          ⛁ ${confs} ${confs === 1 ? 'Confirmation' : 'Confirmations'}
        </span>
        ${data.block_height !== null && data.block_height !== undefined ? `
          <span class="take-modal-pill-badge" style="color: var(--tru-gold); font-weight: 600;">
            Block #${data.block_height}
          </span>
        ` : ''}
      </div>
    </div>

    <!-- Inputs & Outputs -->
    <div class="tru-tx-io-grid">
      <div class="tru-tx-io-col">
        <div class="tru-tx-io-title inputs">
          <span>↓ Inputs</span>
        </div>
        <div class="tru-tx-io-list">
          ${inputsHtml}
        </div>
      </div>
      <div class="tru-tx-io-col">
        <div class="tru-tx-io-title outputs">
          <span>↑ Outputs</span>
        </div>
        <div class="tru-tx-io-list">
          ${outputsHtml}
        </div>
      </div>
    </div>

    <!-- Metadata Grid -->
    <div class="tru-tx-meta-box">
      <div class="tru-tx-meta-title">Metadata & Telemetry</div>
      <div class="tru-tx-meta-grid">
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Block Height:</span>
          <span class="tru-tx-meta-val highlight-gold">${blockHeight}</span>
        </div>
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Confirmations:</span>
          <span class="tru-tx-meta-val">${confs} ${confs === 1 ? 'Confirmation' : 'Confirmations'}</span>
        </div>
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Lock Time:</span>
          <span class="tru-tx-meta-val">${data.lock_time || 0}</span>
        </div>
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Version:</span>
          <span class="tru-tx-meta-val">${data.version || 1}</span>
        </div>
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Is Coinbase:</span>
          <span class="tru-tx-meta-val">${data.is_coinbase ? 'Yes' : 'No'}</span>
        </div>
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Block Hash:</span>
          <span class="tru-tx-meta-val" title="${escapeHtml(blockHash)}">
            ${escapeHtml(shortBlockHash)}
            ${blockHash && blockHash !== '—' ? `<button type="button" class="mecha-mini-btn" style="height: 18px; padding: 0 4px; font-size: 0.65rem; margin-left: 4px;" onclick="copyTxToClipboard('${escapeHtml(blockHash)}', event)" title="Copy Block Hash">📋</button>` : ''}
          </span>
        </div>
        <div class="tru-tx-meta-row full-width">
          <span class="tru-tx-meta-label">Timestamp:</span>
          <span class="tru-tx-meta-val">${data.formatted_time || 'Recent'}</span>
        </div>
      </div>
    </div>
  `;
}

window.openTruTxModal = openTruTxModal;
window.closeTruTxModal = closeTruTxModal;

function renderInProgressRows() {
  updateTableHeaders('inprogress');
  const tbody = document.getElementById('orderbook-rows');
  if (!tbody) return;

  const activeDeals = getActiveInProgressSwaps();
  if (activeDeals.length === 0) {
    tbody.innerHTML = `<tr><td colspan="6" style="text-align: center; color: var(--text-dim); padding: 40px;">No swaps currently in progress. Take a Buy or Sell order to initiate an atomic swap!</td></tr>`;
    return;
  }

  tbody.innerHTML = activeDeals.map(s => {
    const isBuy = s.direction === 2;
    const sideBadge = isBuy
      ? `<span class="tracker-deal-badge-side side-buy" style="font-size: 0.7rem;">BUY TRU</span>`
      : `<span class="tracker-deal-badge-side side-sell" style="font-size: 0.7rem;">SELL TRU</span>`;
    const truAmt = Number(s.amount_tru) || 0;
    const usdcAmt = Number(s.amount_usdc) || truAmt;
    const confs = Number(s.confirmations) || 0;
    const isSafe = s.is_safe || confs >= 6;
    const timeDisplay = s.hours_remaining ? `~${s.hours_remaining.toFixed(1)}h` : 'Active';
    const cleanHash = (s.hash160_hex || '').replace(/^0x/, '').toLowerCase();
    const targetSwapId = s.base_swap_id || '';
    const hasBaseSwap = targetSwapId && targetSwapId.startsWith('0x') && targetSwapId.length === 66;

    const baseLockTx = s.base_lock_tx || '';
    const truLockTx = s.txid || '';
    let txBadges = '';
    if (baseLockTx) {
      const shortB = baseLockTx.startsWith('0x') ? `${baseLockTx.slice(0, 6)}...${baseLockTx.slice(-4)}` : `0x${baseLockTx.slice(0, 4)}...${baseLockTx.slice(-4)}`;
      txBadges += `<a href="${BASE_EXPLORER_URL}/tx/${baseLockTx}" target="_blank" class="tracker-step-tx tracker-link-base" style="font-size: 0.65rem; padding: 1px 4px;" title="Base Lock TX" onclick="event.stopPropagation()">Base: ${shortB} ↗</a> `;
    }
    if (truLockTx) {
      const shortT = `${truLockTx.slice(0, 6)}...${truLockTx.slice(-4)}`;
      txBadges += `<span class="tracker-step-tx tracker-link-tru copyable-tx" style="font-size: 0.65rem; padding: 1px 4px; cursor: pointer;" onclick="openTruTxModal('${truLockTx}', event)" title="View TRU Transaction in Built-in Explorer">TRU: ${shortT} 🔍</span>`;
    }

    let confStatus = isSafe
      ? `<span style="color: #10b981; font-weight: 700;">✅ ${confs}/6 [SAFE]</span>`
      : `<span style="color: #fbbf24; font-weight: 700;">⏳ ${confs}/6 blocks (${Math.round((confs / 6) * 100)}%)</span>`;

    let actionBtn = isSafe ? '⚡ Auto-claiming' : '⏳ Confirming';
    if (isBuy && s.is_mine && !hasBaseSwap) {
      if (confs >= 3) {
        actionBtn = `<button class="btn-action" style="background: linear-gradient(135deg, #3b82f6, #1d4ed8); color: white; padding: 4px 8px; font-size: 0.72rem; border-radius: 4px; border: none; cursor: pointer;" onclick="depositBaseHtlcForBuy('${cleanHash}', ${usdcAmt}, '${s.taker_base || ''}', '${s.txid || ''}', ${s.vout || 1}, ${truAmt})" title="Manual deposit allowed (>=3 TRU confs): Lock ${usdcAmt.toFixed(2)} USDC on Base">Lock USDC</button>`;
      } else {
        actionBtn = `<span style="color: #94a3b8; font-size: 0.72rem; font-family: var(--font-mono);">⏳ Confirming (${confs}/3)</span>`;
      }
    } else if (isBuy && s.is_mine && isSafe) {
      actionBtn = `<button class="btn-action" style="background: linear-gradient(135deg, #10b981, #059669); color: white; padding: 4px 8px; font-size: 0.72rem; border-radius: 4px; border: none; cursor: pointer;" onclick="claimTruHtlc('${cleanHash}', '${s.txid || ''}', ${s.vout || 1}, ${truAmt})">Claim TRU</button>`;
    }

    return `
      <tr class="ob-history-row" style="background: rgba(0, 240, 255, 0.04); border-left: 3px solid ${isSafe ? '#10b981' : '#00f0ff'};">
        <td style="padding-left: 14px;">${sideBadge}</td>
        <td>
          ${confStatus}
          ${txBadges ? `<div style="display: flex; gap: 4px; margin-top: 4px; flex-wrap: wrap;">${txBadges}</div>` : ''}
        </td>
        <td style="text-align: right; font-family: var(--font-mono); font-weight: 700; color: #fff;">${truAmt.toFixed(4)}</td>
        <td style="text-align: right; font-family: var(--font-mono); font-weight: 700; color: var(--tru-gold);">${usdcAmt.toFixed(2)}</td>
        <td style="font-family: var(--font-mono); color: #94a3b8;">${timeDisplay}</td>
        <td style="text-align: center;">${actionBtn}</td>
      </tr>
    `;
  }).join('');
}

function setOrderFilter(filter) {
  currentFilter = filter;
  document.querySelectorAll('.filter-tabs .tab-btn').forEach(btn => btn.classList.remove('active'));
  const activeBtn = document.getElementById('tab-' + filter);
  if (activeBtn) activeBtn.classList.add('active');

  const historyContainer = document.getElementById('trade-history-container');
  const tableContainer = document.querySelector('.table-container.orderbook-frame');
  const trackerEl = document.getElementById('active-swaps-tracker');

  if (filter === 'history' || filter === 'ledger') {
    if (tableContainer) tableContainer.style.display = 'none';
    if (trackerEl) trackerEl.style.display = 'none';
    if (historyContainer) historyContainer.style.display = 'flex';
    renderTradeHistory();
  } else {
    if (historyContainer) historyContainer.style.display = 'none';
    if (tableContainer) tableContainer.style.display = '';

    if (filter === 'mempool') {
      renderMempoolRows(mempoolData.transactions);
      fetchMempoolData();
    } else if (filter === 'inprogress') {
      renderInProgressRows();
    } else {
      renderOrderbook();
    }
    renderActiveSwapsTracker();
  }
}

function fetchMempoolData() {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ action: "req_mempool" }));
  }
}

function renderMempoolRows(txs) {
  updateTableHeaders('mempool');
  const tbody = document.getElementById('orderbook-rows');
  if (!tbody) return;
  if (!txs || txs.length === 0) {
    tbody.innerHTML = `<tr><td colspan="8" style="text-align: center; color: var(--text-dim); padding: 35px;">Mempool is currently empty. Listening for incoming transactions...</td></tr>`;
    return;
  }

  const nowSec = Math.floor(Date.now() / 1000);
  tbody.innerHTML = txs.map(t => {
    let typeBadge = '';
    if (t.is_htlc) {
      typeBadge = `<span class="ob-badge-capsule ob-capsule-htlc">🔒 ATOMIC HTLC</span>`;
    } else if (t.is_vault_related) {
      typeBadge = `<span class="ob-badge-capsule ob-capsule-vault">⚡ VAULT</span>`;
    } else {
      typeBadge = `<span class="ob-badge-capsule ob-capsule-p2p">P2P TRANSFER</span>`;
    }

    const elapsedSec = Math.max(0, nowSec - (t.first_seen_sec || nowSec));
    const netAmount = (t.target_amount_tru !== undefined && t.target_amount_tru > 0) ? t.target_amount_tru : t.total_out_tru;

    let changeDesc = `<span class="ob-meta-cell">${t.outputs_count} out</span>`;
    if (t.change_amount_tru && t.change_amount_tru > 0) {
      changeDesc = `<span class="ob-meta-cell">${t.outputs_count} out <span style="color: rgba(255,255,255,0.7);">(+${t.change_amount_tru.toFixed(2)} chg)</span></span>`;
    }

    return `
      <tr>
        <td style="text-align: left; padding-left: 12px;">${typeBadge}</td>
        <td><span class="ob-net-channel"><strong>Mempool</strong></span></td>
        <td><span class="ob-amount-tru">${netAmount.toFixed(4)} TRU</span></td>
        <td>${changeDesc}</td>
        <td class="ob-meta-cell">${t.inputs_count} in (${t.size_bytes || Math.max(180, (t.inputs_count * 148) + (t.outputs_count * 34) + 10)} B)</td>
        <td class="mempool-first-seen-cell ob-meta-cell" data-first-seen="${t.first_seen_sec || nowSec}">${elapsedSec}s ago</td>
        <td><span class="ob-txid-link copyable-tx" style="cursor: pointer;" onclick="openTruTxModal('${t.txid}', event)" title="View in Built-in Explorer">${t.txid.slice(0, 8)}...${t.txid.slice(-6)} 🔍</span></td>
        <td><span class="ob-status-capsule status-awaiting">⏳ AWAITING BLOCK</span></td>
      </tr>
    `;
  }).join('');
}

// Lightweight local 1s timer to dynamically tick countdowns
setInterval(() => {
  const nowSec = Math.floor(Date.now() / 1000);
  if (currentFilter === 'mempool') {
    document.querySelectorAll('.mempool-first-seen-cell').forEach(el => {
      const fs = parseInt(el.dataset.firstSeen || '0', 10);
      if (fs > 0) {
        const age = Math.max(0, nowSec - fs);
        el.textContent = `${age}s ago`;
      }
    });
  } else {
    document.querySelectorAll('.orderbook-expiry-cell').forEach(el => {
      const tl = parseInt(el.dataset.timelock || '0', 10);
      if (tl > 0) {
        el.innerHTML = formatRemainingTime(tl);
      }
    });
  }
}, 1000);

function updateScannerTelemetry() {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ action: "req_swaps_ledger" }));
  }
}

function fetchOrderbookFromApi() {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ action: "req_orderbook" }));
    ws.send(JSON.stringify({ action: "req_swaps_ledger" }));
  }
}

// --- Reactive Orderbook State Management (Fed by Local Backend WebSocket) ---

function applyOrderAdded(o) {
  if (!o) return;
  const tru = Number(o.amount_tru) || 0;
  const usdc = Number(o.amount_usdc) || 0;
  if (usdc <= 0 && tru <= 0) return;
  const cleanHash = o.hash160_hex ? o.hash160_hex.replace(/^0x/, '').toLowerCase() : (o.hash160 ? o.hash160.replace(/^0x/, '').toLowerCase() : '');
  const canonicalId = cleanHash || o.order_id || o.id;

  const order = {
    id: o.order_id || o.id || canonicalId,
    side: (o.side === 'BuyTruWithBaseUsdc' || o.side === 'BUY_TRU') ? 'BUY_TRU' : 'SELL_TRU',
    truAmount: tru,
    usdcAmount: usdc,
    price: (tru > 0) ? (usdc / tru) : (o.price || 1.0),
    total: usdc,
    hash160: cleanHash,
    timelock: o.timelock,
    isLive: !o.is_matched && !o.taker_base_address && o.is_active !== false,
    isMatched: !!o.is_matched || !!(o.taker_base_address && o.taker_base_address.length > 0),
    isFundedOnBase: o.source_chain === 'Base' || o.source_chain === 'Base Sepolia' || (o.source_chain && o.source_chain.startsWith('Base')),
    baseSwapId: ((o.source_chain === 'Base' || o.source_chain === 'Base Sepolia' || (o.source_chain && o.source_chain.startsWith('Base'))) ? (o.id || o.order_id) : ''),
    sourceChain: o.source_chain || 'TRUSwap-Relay',
    makerBaseAddress: o.maker_base_address,
    makerTruAddress: o.maker_tru_address || '',
    makerTruPubkey: o.maker_tru_pubkey || (o.maker_tru_address && o.maker_tru_address.length === 66 ? o.maker_tru_address : null),
  };
  ordersMap.set(canonicalId, order);
  renderActiveSwapsTracker();
  renderOrderbook();
}

function applyOrderRemoved(orderId) {
  if (!orderId) return;
  const clean = String(orderId).replace(/^0x/, '').toLowerCase();
  ordersMap.delete(clean);
  for (const [k, v] of ordersMap.entries()) {
    if (v.id === clean || v.hash160 === clean || k === clean) {
      ordersMap.delete(k);
    }
  }
  if (myCreatedOrders && myCreatedOrders.has(clean)) {
    myCreatedOrders.delete(clean);
    try {
      localStorage.setItem('my_truswap_orders', JSON.stringify(Array.from(myCreatedOrders)));
    } catch (e) { }
  }
  renderActiveSwapsTracker();
  renderOrderbook();
}

function applyOrderMatched(data) {
  if (!data) return;
  const cleanId = (data.order_id || '').replace(/^0x/, '').toLowerCase();
  const cleanHash = (data.hash160 || '').replace(/^0x/, '').toLowerCase();
  for (const [k, v] of ordersMap.entries()) {
    if (v.id === cleanId || v.hash160 === cleanHash || k === cleanId || k === cleanHash) {
      v.isMatched = true;
      v.isLive = false;
      if (data.taker_base) v.takerBaseAddress = data.taker_base;
      if (data.taker_tru) v.takerTruPubkey = data.taker_tru;
    }
  }
  renderActiveSwapsTracker();
  renderOrderbook();
}

function applyOrderbookSnapshot(orders) {
  ordersMap.clear();
  const activeSwaps = getActiveInProgressSwaps();
  const inProgressHashes = new Set(activeSwaps.map(s => (s.hash160_hex || '').replace(/^0x/, '').toLowerCase()));
  const inProgressIds = new Set(activeSwaps.map(s => (s.txid || s.base_swap_id || s.id || '').replace(/^0x/, '').toLowerCase()));

  if (Array.isArray(orders)) {
    for (const o of orders) {
      const cleanH = (o.hash160_hex || o.hash160 || '').replace(/^0x/, '').toLowerCase();
      const cleanId = (o.order_id || o.id || '').replace(/^0x/, '').toLowerCase();
      if (inProgressHashes.has(cleanH) || inProgressIds.has(cleanId)) {
        continue;
      }
      applyOrderAdded(o);
    }
  }
  renderActiveSwapsTracker();
  renderOrderbook();
}

// Auto-sync orderbook and swaps ledger on load and periodically as background fallback
setTimeout(fetchOrderbookFromApi, 100);
setInterval(fetchOrderbookFromApi, 5000);

// Global exports for inline HTML onclick handlers and socket.js
window.setOrderFilter = setOrderFilter;
window.renderOrderbook = renderOrderbook;
window.renderTradeHistory = renderTradeHistory;
window.renderMempoolRows = renderMempoolRows;
window.fetchOrderbookFromApi = fetchOrderbookFromApi;
window.applyOrderAdded = applyOrderAdded;
window.applyOrderRemoved = applyOrderRemoved;
window.applyOrderMatched = applyOrderMatched;
window.applyOrderbookSnapshot = applyOrderbookSnapshot;

// ============================================================================
// AUTOMATION LOGS & AUDIT MODAL (PROCESS OVERLAY)
// ============================================================================
window.automationLogs = [];
let activeAuditModalHash = null;
let auditAutoScroll = true;

function isLogMatchingSwap(line, swap, cleanHash) {
  if (!line || !cleanHash) return false;
  const l = line.toLowerCase();
  const shortH = cleanHash.slice(0, 8);

  const orderId = String((swap && (swap.id || swap.order_id)) || '').replace(/^0x/, '').toLowerCase();
  const shortOrder = orderId ? orderId.slice(0, 8) : '';
  const truTx = String((swap && swap.txid) || '').replace(/^0x/, '').toLowerCase();
  const shortTruTx = truTx ? truTx.slice(0, 16) : '';
  const baseSwapId = String((swap && swap.base_swap_id) || '').replace(/^0x/, '').toLowerCase();
  const shortBaseSwap = baseSwapId ? baseSwapId.slice(0, 8) : '';
  const baseLockTx = String((swap && swap.base_lock_tx) || '').replace(/^0x/, '').toLowerCase();
  const shortBaseTx = baseLockTx ? baseLockTx.slice(0, 10) : '';
  const claimTx = String((swap && (swap.spent_by_txid || swap.claim_txid)) || '').replace(/^0x/, '').toLowerCase();
  const shortClaimTx = claimTx ? claimTx.slice(0, 16) : '';

  // If the log explicitly mentions a different hash160, strictly reject
  const hashMatch = l.match(/hash160[=:\s]+0x?([a-f0-9]{8,})/i) || l.match(/\(hash(?:160)?:?\s*0x?([a-f0-9]{8,})\)/i) || l.match(/\(0x([a-f0-9]{8,})\.\.\.\)/i) || l.match(/hash:\s*0x?([a-f0-9]{8,})/i);
  if (hashMatch && hashMatch[1]) {
    const refH = hashMatch[1].toLowerCase();
    if (!refH.startsWith(shortH) && !shortH.startsWith(refH.slice(0, 8))) {
      return false;
    }
  }

  // If the log explicitly mentions a different order id, reject
  const orderMatch = l.match(/order\s+0x?([a-f0-9]{8,})/i);
  if (orderMatch && orderMatch[1] && shortOrder) {
    const refO = orderMatch[1].toLowerCase();
    if (!refO.startsWith(shortOrder) && !shortOrder.startsWith(refO.slice(0, 8))) {
      return false;
    }
  }

  // Match identifiers
  if (shortH && l.includes(shortH)) return true;
  if (shortOrder && l.includes(shortOrder)) return true;
  if (shortTruTx && l.includes(shortTruTx)) return true;
  if (shortBaseSwap && l.includes(shortBaseSwap)) return true;
  if (shortBaseTx && l.includes(shortBaseTx)) return true;
  if (shortClaimTx && l.includes(shortClaimTx)) return true;

  return false;
}

function getLogsForSwap(swapOrHash) {
  if (!swapOrHash) return [];
  const cleanH = typeof swapOrHash === 'string'
    ? swapOrHash.replace(/^0x/, '').toLowerCase()
    : String(swapOrHash.hash160_hex || swapOrHash.hash160 || '').replace(/^0x/, '').toLowerCase();

  const swap = (typeof swapOrHash === 'object' && swapOrHash !== null)
    ? swapOrHash
    : findAuditModalSwap(cleanH);

  const logs = Array.isArray(window.automationLogs) ? window.automationLogs : [];
  return logs.filter(l => isLogMatchingSwap(l, swap, cleanH));
}

function handleAutomationLog(data) {
  if (!data) return;
  const raw = typeof data === 'string' ? data : (data.entry || data.msg || '');
  const entry = String(raw || '').trim();
  if (!entry) return;

  // Deduplicate exact consecutive events
  if (window.automationLogs.length > 0) {
    const last = window.automationLogs[window.automationLogs.length - 1];
    if (last === entry) {
      return;
    }
  }

  window.automationLogs.push(entry);
  if (window.automationLogs.length > 350) {
    window.automationLogs.shift();
  }

  // Update in-progress mini-logs for matching active swaps
  const activeSwaps = typeof getActiveInProgressSwaps === 'function' ? getActiveInProgressSwaps() : [];
  activeSwaps.forEach(s => {
    const cleanHash = String(s.hash160_hex || s.hash160 || '').replace(/^0x/, '').toLowerCase();
    const previewEl = document.getElementById(`deal-log-preview-${cleanHash}`);
    if (previewEl) {
      previewEl.innerText = getLatestAutomationLogForSwap(s);
    }
  });

  // If audit modal is currently open, append line in real time ONLY IF it belongs to activeAuditModalHash
  const consoleEl = document.getElementById('audit-modal-console');
  if (consoleEl && activeAuditModalHash) {
    const swap = findAuditModalSwap(activeAuditModalHash);
    if (isLogMatchingSwap(entry, swap, activeAuditModalHash)) {
      if (consoleEl.innerHTML.includes('No telemetry events logged yet') || consoleEl.innerHTML.includes('No daemon events logged yet')) {
        consoleEl.innerHTML = '';
      }
      const lineEl = document.createElement('div');
      lineEl.className = 'audit-log-line ' + getLogLineClass(entry);
      lineEl.innerHTML = formatAuditLogLine(entry);
      consoleEl.appendChild(lineEl);
      if (auditAutoScroll) {
        consoleEl.scrollTop = consoleEl.scrollHeight;
      }
      if (typeof updateAuditModalTimeline === 'function') {
        updateAuditModalTimeline();
      }
    }
  }
}

function handleAutomationLogsList(logs) {
  if (Array.isArray(logs)) {
    const cleaned = [];
    for (const item of logs) {
      const s = String(item || '').trim();
      if (!s) continue;
      if (cleaned.length === 0 || cleaned[cleaned.length - 1] !== s) {
        cleaned.push(s);
      }
    }
    window.automationLogs = cleaned.slice(-350);

    const activeSwaps = typeof getActiveInProgressSwaps === 'function' ? getActiveInProgressSwaps() : [];
    activeSwaps.forEach(s => {
      const cleanHash = String(s.hash160_hex || s.hash160 || '').replace(/^0x/, '').toLowerCase();
      const previewEl = document.getElementById(`deal-log-preview-${cleanHash}`);
      if (previewEl) {
        previewEl.innerText = getLatestAutomationLogForSwap(s);
      }
    });

    if (activeAuditModalHash) {
      renderAutomationConsole();
      if (typeof updateAuditModalTimeline === 'function') {
        updateAuditModalTimeline();
      }
    }
  }
}

function getLogLineClass(line) {
  if (line.includes('⚡') || line.includes('🎉') || line.includes('✔') || line.includes('Confirmed') || line.includes('Complete')) {
    return 'log-success';
  }
  if (line.includes('⏳') || line.includes('Waiting') || line.includes('Awaiting')) {
    return 'log-warn';
  }
  if (line.includes('⚠️') || line.includes('Failed') || line.includes('error')) {
    return 'log-error';
  }
  return 'log-info';
}

function getLatestAutomationLogForSwap(s) {
  const swapLogs = getLogsForSwap(s);
  if (swapLogs.length > 0) {
    const raw = String(swapLogs[swapLogs.length - 1] || '');
    // Strip leading bracketed timestamps like [21:18:19] for clean mini preview
    let cleaned = raw.replace(/^\[\d{1,2}:\d{2}(?::\d{2})?\]\s*/, '').trim();
    // Compact verbose repeated phrases so preview fits cleanly on mobile screens
    cleaned = cleaned
      .replace(/^⏳?\s*\[(?:Seller|Buyer|P2P|Auto)\s*Guard\]\s*/i, '⏳ ')
      .replace(/\s+on Base HTLC Contract/gi, ' on Base')
      .replace(/\s+on Base HTLC/gi, ' on Base')
      .replace(/\s*\(0x[a-fA-F0-9]{3,}\.\.\.[^)]*\)/g, '')
      .replace(/,\s*waiting for Buyer Base USDC lock\.?/i, ' -> wait USDC')
      .replace(/Waiting for Buyer Base USDC lock/i, 'Waiting for USDC')
      .replace(/TRU lock broadcasted/i, 'TRU locked')
      .replace(/\s*Continuous chain verification active\.?/gi, '')
      .trim();
    return cleaned;
  }

  // Fallback to state-derived status
  const confs = Number(s.confirmations) || 0;
  if (s.status === 'Claimed' || s.is_spent || s.base_claimed) {
    return `🎉 Swap Settled & Finalized`;
  }
  if (confs >= 6) {
    return `⚡ 6/6 TRU Safe (Settling)`;
  }
  if (confs > 0) {
    return `⏳ Confirmed: ${confs}/6 blocks`;
  }
  if (s.txid) {
    return `⛏️ Mining TRU block...`;
  }
  return `🤖 Engine Active`;
}

function ensureAutomationAuditModalDom() {
  if (document.getElementById('automation-audit-modal')) return;

  const modalHtml = `
    <div id="automation-audit-modal" onclick="if(event.target === this) closeAutomationAuditModal()">
      <div class="audit-modal-dialog">
        <div class="audit-modal-led-bar">
          <span class="led-pill led-pill-1"></span>
          <span class="led-pill led-pill-2"></span>
          <span class="led-pill led-pill-3"></span>
          <span class="led-pill led-pill-4"></span>
          <span class="led-pill led-pill-5"></span>
        </div>
        <div class="audit-modal-header">
          <div class="audit-header-title-group">
            <span class="audit-header-icon">🤖</span>
            <div>
              <div class="audit-header-title">P2P Autonomous Engine | Process Audit</div>
              <div class="audit-header-subtitle" id="audit-modal-subtitle">Trustless Atomic Swap State & Telemetry</div>
            </div>
          </div>
          <button type="button" class="audit-modal-close-btn" onclick="closeAutomationAuditModal()" title="Close (Esc)">✕</button>
        </div>

        <div class="audit-modal-body">
          <div class="audit-timeline">
            <div class="audit-timeline-title">
              <span>Automated Execution Pipeline (5 Stages)</span>
              <span id="audit-stage-status-badge" style="font-family: var(--font-mono); color: #00c8e0; font-weight: 800;">ACTIVE</span>
            </div>
            <div id="audit-timeline-steps"></div>
          </div>

          <div class="audit-console-wrap">
            <div class="audit-console-bar">
              <span class="audit-console-heading">
                <span class="tracker-automation-pulse"></span>
                Event Stream
              </span>
              <div class="audit-console-tools">
                <button type="button" class="audit-tool-btn" id="audit-btn-autoscroll" onclick="toggleAutoScrollLogs()">Auto-Scroll: ON</button>
                <button type="button" class="audit-tool-btn" onclick="copyAutomationLogsToClipboard()">📋 Copy Logs</button>
              </div>
            </div>
            <div class="audit-console" id="audit-modal-console"></div>
          </div>
        </div>

        <div class="audit-modal-footer">
          <div class="audit-footer-status">
            TRU SPV &amp; BaseHTLC Watcher
          </div>
        </div>
      </div>
    </div>
  `;

  document.body.insertAdjacentHTML('beforeend', modalHtml);

  // Esc key listener
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && activeAuditModalHash) {
      closeAutomationAuditModal();
    }
  });
}

function findAuditModalSwap(hashClean) {
  if (!hashClean) return {};
  const cleanH = String(hashClean).replace(/^0x/, '').toLowerCase();

  const allLedger = (typeof historicalLedgerData !== 'undefined' && historicalLedgerData && Array.isArray(historicalLedgerData.swaps)) ? historicalLedgerData.swaps : [];
  const fromLedger = allLedger.find(s => (s.hash160_hex || '').replace(/^0x/, '').toLowerCase() === cleanH);
  if (fromLedger) return fromLedger;

  const active = typeof getActiveInProgressSwaps === 'function' ? getActiveInProgressSwaps() : [];
  const fromActive = active.find(s => (s.hash160_hex || '').replace(/^0x/, '').toLowerCase() === cleanH);
  if (fromActive) return fromActive;

  if (typeof pendingTakenSwaps !== 'undefined' && pendingTakenSwaps.has(cleanH)) {
    return pendingTakenSwaps.get(cleanH);
  }

  if (typeof ordersMap !== 'undefined' && ordersMap) {
    for (const [k, v] of ordersMap.entries()) {
      if ((v.hash160 || '').toLowerCase() === cleanH || (v.id || '').toLowerCase() === cleanH || k.toLowerCase() === cleanH) {
        return {
          hash160_hex: v.hash160 || cleanH,
          id: v.id || k,
          direction: v.direction || (v.side === 'BUY' ? 2 : 1),
          amount_tru: v.amountTru || v.amount_tru || 0,
          amount_usdc: v.amountUsdc || v.amount_usdc || 0,
          txid: v.txid || '',
          base_swap_id: v.baseSwapId || v.base_swap_id || '',
          base_lock_tx: v.baseLockTx || v.base_lock_tx || '',
          beneficiary: v.makerBaseAddress || v.takerBaseAddress || '',
          confirmations: v.confirmations || 0,
          status: v.status || (v.isMatched ? 'Matched' : 'Open'),
        };
      }
    }
  }

  return {};
}

function updateAuditModalTimeline() {
  if (!activeAuditModalHash) return;
  const modal = document.getElementById('automation-audit-modal');
  if (!modal || (!modal.classList.contains('open') && modal.style.display !== 'flex')) return;

  const currentSwap = findAuditModalSwap(activeAuditModalHash);

  const subEl = document.getElementById('audit-modal-subtitle');
  if (subEl) {
    const side = currentSwap.direction === 2 ? 'BUY TRU' : 'SELL TRU';
    const truAmt = Number(currentSwap.amount_tru) || 0;
    const usdcAmt = Number(currentSwap.amount_usdc) || truAmt;
    if (truAmt > 0) {
      subEl.innerText = `${side} (${truAmt.toFixed(4)} TRU ↔ ${usdcAmt.toFixed(2)} USDC) | Hash: 0x${activeAuditModalHash.slice(0, 10)}...`;
    }
  }

  renderAutomationTimeline(currentSwap);
}

function openAutomationAuditModal(hash160) {
  ensureAutomationAuditModalDom();
  activeAuditModalHash = String(hash160 || '').replace(/^0x/, '').toLowerCase();

  const modal = document.getElementById('automation-audit-modal');
  if (!modal) return;

  modal.classList.add('open');
  modal.style.display = 'flex';
  document.body.classList.add('modal-open');

  updateAuditModalTimeline();
  renderAutomationConsole();
}

function closeAutomationAuditModal() {
  const modal = document.getElementById('automation-audit-modal');
  if (modal) {
    modal.classList.remove('open');
    modal.style.display = '';
  }
  document.body.classList.remove('modal-open');
  activeAuditModalHash = null;
}

function renderAutomationTimeline(s) {
  const stepsContainer = document.getElementById('audit-timeline-steps');
  if (!stepsContainer) return;

  s = s || {};
  const cleanH = String(s.hash160_hex || activeAuditModalHash || '').replace(/^0x/, '').toLowerCase();
  const shortH = cleanH.slice(0, 8);

  let isSettled = Boolean(s.status === 'Claimed' || s.is_spent || s.spent_by_txid || s.base_claimed || s.vault_status === 1 || s.vault_step === 4);
  let hasLockedBase = Boolean(s.base_swap_id && s.base_swap_id.startsWith('0x') && s.base_swap_id.length === 66);
  let hasSafeTru = Boolean(s.is_safe || (Number(s.confirmations) >= 6));
  let truTx = s.txid || '';
  let baseTx = s.base_lock_tx || '';

  // Cross-reference live automation logs strictly for this swap
  const relevantLogs = getLogsForSwap(cleanH);
  if (Array.isArray(relevantLogs) && shortH) {
    for (const l of relevantLogs) {
      if (l.includes('Atomic Swap Complete') || l.includes('Sweeping TRU funds')) {
        isSettled = true;
        hasLockedBase = true;
        hasSafeTru = true;
      }
      if (l.includes('Claiming') && l.includes('from HTLC: TXID')) {
        const m = l.match(/TXID\s+([a-fA-F0-9]{64})/);
        if (m && (!truTx || !s.txid)) truTx = m[1];
      }
      if (l.includes('USDC Locked on Base!')) {
        hasLockedBase = true;
        const m = l.match(/TX:\s+(0x[a-fA-F0-9]{64})/);
        if (m && !baseTx) baseTx = m[1];
      }
      if (l.includes('Seller TRU Finalized (6/6 SAFE)!')) {
        hasSafeTru = true;
      }
    }
  }

  if (isSettled) {
    hasLockedBase = true;
    hasSafeTru = true;
  }

  const confs = Number(s.confirmations) || (hasSafeTru ? 6 : 0);
  const isSafe = hasSafeTru || confs >= 6;
  const isMined = isSafe || Boolean((s.block_height && s.block_height > 0) || confs > 0);
  const isBroadcasted = isMined || Boolean(truTx);
  const hasBaseSwap = hasLockedBase;

  // Update status badge
  const badgeEl = document.getElementById('audit-stage-status-badge');
  if (badgeEl) {
    if (isSettled) {
      badgeEl.innerText = 'SETTLED';
      badgeEl.style.color = '#10b981';
    } else {
      badgeEl.innerText = 'ACTIVE';
      badgeEl.style.color = '#38bdf8';
    }
  }

  const isBuy = s.direction === 2;
  const hasSecret = Boolean(s.has_secret || (s.secret && !s.secret.startsWith('00000000') && s.secret.length === 64));
  const isBaseFirst = (isBuy === hasSecret) || (hasBaseSwap && !isSafe);
  const isMakerBuyer = isBuy && hasSecret;
  const isTakerBuyer = isBuy && !hasSecret;
  const usdcAmt = Number(s.amount_usdc || (s.amount_tru ? s.amount_tru * 4 : 0));
  const truAmt = Number(s.amount_tru || 0);
  const counterpartyBase = String(s.beneficiary || s.maker_base_address || s.taker_base || '').trim();
  const canManualLockBase = !hasBaseSwap && isBuy && confs >= 3 && !isSettled;
  const canManualClaimTru = hasBaseSwap && !isSettled && isBuy && confs >= 3;

  // Step Descriptions & Statuses
  let confDesc = '';
  if (isSettled) {
    confDesc = confs >= 6
      ? `✅ Achieved 6/6 confirmations! Trustless settlement finalized.`
      : `✅ Fast Settlement: Verified on-chain (${confs}/6 blocks) and settled early.`;
  } else if (confs >= 6) {
    confDesc = `✅ Achieved 6/6 confirmations! Verified safe on TRU chain.`;
  } else {
    confDesc = `⏳ Current: ${confs}/6 blocks confirmed (${Math.round((Math.min(confs, 6) / 6) * 100)}%).`;
  }
  const confStatus = isSettled
    ? (confs >= 6 ? "SAFE (6/6)" : `EARLY (${confs}/6)`)
    : (confs >= 6 ? "SAFE (6/6)" : (isMined ? `CONFIRMING (${confs}/6)` : "WAITING"));
  const confClass = (isSettled || confs >= 6) ? "completed" : (isMined ? "active" : "");

  let baseLockDesc = '';
  if (hasBaseSwap) {
    baseLockDesc = `BaseHTLC Contract: <a href="${BASE_EXPLORER_URL}/address/${BASE_HTLC_ADDR}" target="_blank" style="color: #60a5fa;">${BASE_HTLC_ADDR.slice(0, 8)}... ↗</a> ${baseTx ? `| TX: <a href="${BASE_EXPLORER_URL}/tx/${baseTx}" target="_blank" style="color: #60a5fa;">${baseTx.slice(0, 10)}... ↗</a>` : ''}`;
  } else if (isBaseFirst) {
    if (isBuy) {
      baseLockDesc = `⏳ Locking ${usdcAmt.toFixed(2)} USDC on BaseHTLC (Zero-Trust Initiator)...`;
    } else {
      baseLockDesc = `⏳ Awaiting Buyer deposit of ${usdcAmt.toFixed(2)} USDC on BaseHTLC...`;
    }
  } else if (isTakerBuyer) {
    if (isSafe) {
      baseLockDesc = `⏳ 6/6 Confirmed! Autonomous Buyer Safety Guard locking ${usdcAmt.toFixed(2)} USDC on Base...`;
    } else if (canManualLockBase) {
      baseLockDesc = `🔒 Safety Lock: Buyer holds USDC until 6 confs.<br><div style="margin-top: 6px;"><button class="btn-action" style="background: linear-gradient(135deg, #3b82f6, #1d4ed8); color: white; border: none; font-weight: 700; padding: 4px 10px; border-radius: 6px; cursor: pointer; font-size: 0.75rem;" onclick="depositBaseHtlcForBuy('${cleanH}', ${usdcAmt}, '${counterpartyBase}', '${truTx}', ${s.vout || 1}, ${truAmt})" title="Manual deposit allowed (>=3 TRU confs): Lock ${usdcAmt.toFixed(2)} USDC on Base">⚡ Manual Override: Lock ${usdcAmt.toFixed(2)} USDC (${confs}/3 confs ready)</button></div>`;
    } else {
      baseLockDesc = `🔒 Safety Lock: Buyer holds USDC until TRU reaches 6 confirmations.`;
    }
  } else if (isSafe) {
    baseLockDesc = `⏳ Awaiting Buyer deposit of ${usdcAmt.toFixed(2)} USDC on Base now that TRU is confirmed (6/6).`;
  } else {
    baseLockDesc = `🔒 Safety Lock: Buyer holds USDC until TRU reaches 6 confirmations.`;
  }
  const baseLockStatus = hasBaseSwap
    ? "LOCKED"
    : (isBaseFirst ? (isBuy ? "DEPOSITING" : "AWAITING BUYER") : (isTakerBuyer ? (isSafe ? "LOCKING USDC" : "WAITING CONFS") : (isSafe ? "AWAITING BUYER" : "WAITING CONFS")));
  const baseLockClass = hasBaseSwap ? "completed" : ((isBaseFirst || (isTakerBuyer && isSafe) || (!isBuy && isSafe)) ? "active" : "");

  let truLockDesc = truTx
    ? `TRU TXID: <span class="copyable-tx" style="color: #38bdf8; cursor: pointer;" onclick="openTruTxModal('${truTx}', event)">${truTx.slice(0, 16)}...${truTx.slice(-12)} 🔍</span> (${isMined ? 'Confirmed on-chain' : 'Block #' + (s.block_height || 'Mempool')})`
    : (isBuy ? `⏳ Awaiting Seller TRU lock broadcast on TRU chain...` : `Locking ${truAmt.toFixed(4)} TRU on TRU blockchain...`);
  const truLockStatus = isMined ? "MINED" : (isBroadcasted ? "IN MEMPOOL" : (isBuy ? "AWAITING SELLER" : "PENDING"));
  const truLockClass = isMined ? "completed" : (isBroadcasted ? "active" : "");

  let settleDesc = '';
  const canSweep = isSettled || (isSafe && hasBaseSwap) || canManualClaimTru;
  if (isSettled) {
    settleDesc = `🎉 Secret revealed on-chain and funds swept to respective local vaults!`;
  } else if (canManualClaimTru) {
    settleDesc = `⚡ Preimage ready! Autonomous daemon sweeping TRU...<br><div style="margin-top: 6px;"><button class="btn-action" style="background: linear-gradient(135deg, #10b981, #059669); color: white; border: none; font-weight: 700; padding: 4px 10px; border-radius: 6px; cursor: pointer; font-size: 0.75rem;" onclick="claimTruHtlc('${cleanH}', '${truTx}', ${s.vout || 1}, ${truAmt})" title="Manual sweep allowed: Sweep ${truAmt.toFixed(4)} TRU">⚡ Manual Override: Sweep ${truAmt.toFixed(4)} TRU</button></div>`;
  } else if (isSafe && hasBaseSwap) {
    settleDesc = `⚡ Preimage ready! Autonomous daemon sweeping funds...`;
  } else {
    settleDesc = `⏳ Waiting for previous stages to finalize (TRU 6/6 confirmations).`;
  }
  const settleStatus = isSettled ? "SETTLED" : (canSweep ? "SWEEPING" : "WAITING");
  const settleClass = isSettled ? "completed" : (canSweep ? "active" : "");

  let steps = [];
  if (isBaseFirst) {
    // BUYER-INITIATED SWAP (Buyer locks Base USDC first): Base USDC Lock -> Seller TRU Lock -> Confirmations -> Settlement
    steps = [
      {
        num: 1,
        name: "Step 1: Order Matching & Preimage Hash Generation",
        desc: `HTLC Hash160: 0x${s.hash160_hex || activeAuditModalHash} | Direction: ${isBuy ? (hasSecret ? 'BUY TRU (Maker)' : 'BUY TRU (Taker)') : (hasSecret ? 'SELL TRU (Maker)' : 'SELL TRU (Taker)')}`,
        status: "COMPLETED",
        cssClass: "completed"
      },
      {
        num: 2,
        name: `Step 2: ${BASE_NETWORK_NAME} USDC Smart Contract Lock`,
        desc: baseLockDesc,
        status: baseLockStatus,
        cssClass: baseLockClass
      },
      {
        num: 3,
        name: "Step 3: TRU Blockchain HTLC Script Broadcast",
        desc: truLockDesc,
        status: truLockStatus,
        cssClass: truLockClass
      },
      {
        num: 4,
        name: "Step 4: ConfirmationGuard Autonomous Verification",
        desc: confDesc,
        status: confStatus,
        cssClass: confClass
      },
      {
        num: 5,
        name: "Step 5: Preimage Secret Reveal & Trustless Settlement",
        desc: settleDesc,
        status: settleStatus,
        cssClass: settleClass
      }
    ];
  } else {
    // SELLER-INITIATED SWAP (Seller locks TRU first): Seller TRU Lock -> Confirmations -> Base USDC Lock -> Settlement
    steps = [
      {
        num: 1,
        name: "Step 1: Order Matching & Preimage Hash Generation",
        desc: `HTLC Hash160: 0x${s.hash160_hex || activeAuditModalHash} | Direction: ${isBuy ? (hasSecret ? 'BUY TRU (Maker)' : 'BUY TRU (Taker)') : (hasSecret ? 'SELL TRU (Maker)' : 'SELL TRU (Taker)')}`,
        status: "COMPLETED",
        cssClass: "completed"
      },
      {
        num: 2,
        name: "Step 2: TRU Blockchain HTLC Script Broadcast",
        desc: truLockDesc,
        status: truLockStatus,
        cssClass: truLockClass
      },
      {
        num: 3,
        name: "Step 3: ConfirmationGuard Autonomous Verification",
        desc: confDesc,
        status: confStatus,
        cssClass: confClass
      },
      {
        num: 4,
        name: `Step 4: ${BASE_NETWORK_NAME} USDC Smart Contract Lock`,
        desc: baseLockDesc,
        status: baseLockStatus,
        cssClass: baseLockClass
      },
      {
        num: 5,
        name: "Step 5: Preimage Secret Reveal & Trustless Settlement",
        desc: settleDesc,
        status: settleStatus,
        cssClass: settleClass
      }
    ];
  }

  stepsContainer.innerHTML = steps.map(st => `
    <div class="audit-step-row ${st.cssClass}">
      <span class="audit-step-badge">${st.status}</span>
      <div class="audit-step-content">
        <div class="audit-step-name">${st.name}</div>
        <div class="audit-step-desc">${st.desc}</div>
      </div>
    </div>
  `).join('');

  // Update LED bar indicators (matching telemetry spectrum: green, yellow, orange, red, cyan)
  const ledPills = document.querySelectorAll('#automation-audit-modal .audit-modal-led-bar .led-pill');
  if (ledPills && ledPills.length === 5) {
    const colors = ['green', 'yellow', 'orange', 'red', 'cyan'];
    steps.forEach((st, idx) => {
      const isDone = st.cssClass === 'completed';
      const isActive = st.cssClass === 'active';
      const c = colors[idx] || 'cyan';
      ledPills[idx].className = `led-pill led-pill-${idx + 1} ` + (isDone ? `active-${c}` : (isActive ? `active-${c} pulse` : 'dim'));
    });
  }
}

function formatAuditLogLine(raw) {
  if (!raw) return '';
  let escaped = escapeHtml(raw);
  // Highlight timestamp [HH:MM:SS]
  escaped = escaped.replace(/^(\[\d{2}:\d{2}:\d{2}\])/, '<span class="log-ts">$1</span>');
  // Highlight guard and system tags
  escaped = escaped.replace(/(\[[A-Za-z0-9_\s-]+Guard[A-Za-z0-9_\s-]*\]|\[AUTONOMOUS\]|\[ENGINE\]|\[ConfirmationGuard\])/g, '<span class="log-guard">$1</span>');
  // Highlight hex addresses & txids
  escaped = escaped.replace(/(0x[a-fA-F0-9]{8,}|Hash160=[a-fA-F0-9x]+|[a-f0-9]{32,64})/g, '<span class="log-hex">$1</span>');
  return escaped;
}

function renderAutomationConsole() {
  const consoleEl = document.getElementById('audit-modal-console');
  if (!consoleEl) return;

  const relevantLogs = activeAuditModalHash ? getLogsForSwap(activeAuditModalHash) : [];
  if (relevantLogs.length === 0) {
    const cleanH = String(activeAuditModalHash || '').replace(/^0x/, '').toLowerCase();
    const shortH = cleanH.slice(0, 8);
    consoleEl.innerHTML = `<div style="color: #64748b; font-style: italic; padding: 14px 10px;">⏳ No telemetry events logged yet for swap 0x${shortH}... Waiting for events...</div>`;
    return;
  }

  consoleEl.innerHTML = relevantLogs.map(l => `<div class="audit-log-line ${getLogLineClass(l)}">${formatAuditLogLine(l)}</div>`).join('');

  if (auditAutoScroll) {
    consoleEl.scrollTop = consoleEl.scrollHeight;
  }
}

function copyAutomationLogsToClipboard(event) {
  if (event) {
    if (typeof event.preventDefault === 'function') event.preventDefault();
    if (typeof event.stopPropagation === 'function') event.stopPropagation();
  }

  const relevantLogs = activeAuditModalHash ? getLogsForSwap(activeAuditModalHash) : [];
  let text = (Array.isArray(relevantLogs) && relevantLogs.length > 0)
    ? relevantLogs.filter(Boolean).join('\n')
    : '';

  if (!text) {
    const consoleEl = document.getElementById('audit-modal-console');
    if (consoleEl) {
      text = consoleEl.innerText || consoleEl.textContent || '';
    }
  }

  if (!text || text.includes('No telemetry events logged yet') || text.includes('No daemon events logged yet')) {
    if (typeof showToast === 'function') {
      showToast('⚠️ No automation logs available for this swap', 'warning');
    }
    return;
  }

  const btn = event ? (event.currentTarget || event.target) : (document.getElementById('audit-btn-copy') || document.querySelector('.audit-console-tools button:last-child'));
  const originalText = btn ? btn.innerText : '📋 Copy Logs';

  if (typeof copyToClipboard === 'function') {
    copyToClipboard(text, '✅ Automation logs copied to clipboard!');
  } else {
    try {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.style.position = 'fixed';
      ta.style.left = '-9999px';
      ta.style.top = '-9999px';
      ta.setAttribute('readonly', '');
      document.body.appendChild(ta);
      ta.select();
      ta.setSelectionRange(0, 99999);
      const successful = document.execCommand('copy');
      document.body.removeChild(ta);
      if (successful && typeof showToast === 'function') {
        showToast('✅ Automation logs copied to clipboard!');
      }
    } catch (e) {
      console.error('Copy fallback failed:', e);
    }
  }

  if (btn) {
    btn.innerText = '✓ Copied!';
    btn.style.color = '#10b981';
    btn.style.borderColor = 'rgba(16, 185, 129, 0.6)';
    setTimeout(() => {
      btn.innerText = originalText;
      btn.style.color = '';
      btn.style.borderColor = '';
    }, 1800);
  }
}

function toggleAutoScrollLogs() {
  auditAutoScroll = !auditAutoScroll;
  const btn = document.getElementById('audit-btn-autoscroll');
  if (btn) {
    btn.innerText = `Auto-Scroll: ${auditAutoScroll ? 'ON' : 'OFF'}`;
  }
}

// Global exports
window.handleAutomationLog = handleAutomationLog;
window.handleAutomationLogsList = handleAutomationLogsList;
window.openAutomationAuditModal = openAutomationAuditModal;
window.closeAutomationAuditModal = closeAutomationAuditModal;
window.updateAuditModalTimeline = updateAuditModalTimeline;
window.copyAutomationLogsToClipboard = copyAutomationLogsToClipboard;
window.toggleAutoScrollLogs = toggleAutoScrollLogs;

