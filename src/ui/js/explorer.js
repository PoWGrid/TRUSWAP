// TRUSWAP Real-Time Blockchain Explorer Logic
const EXPLORER_BASE = (typeof window !== 'undefined' && window.location && window.location.pathname.startsWith('/explorer')) ? '/explorer' : '';
let currentTipHeight = 0;
let lastSeenBlockHash = '';
let lastBlockTimestamp = 0;
let recentBlocksCache = [];
let mempoolTxsCache = [];

function escapeHtml(str) {
  if (str === null || str === undefined) return '';
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#039;');
}

function timeAgo(epochSec) {
  if (!epochSec || epochSec === 0) return 'Just now';
  const now = Math.floor(Date.now() / 1000);
  const diff = Math.max(0, now - epochSec);
  if (diff < 5) return 'Just now';
  if (diff < 60) return `${diff}s ago`;
  const mins = Math.floor(diff / 60);
  if (mins < 60) return `${mins}m ${diff % 60}s ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ${mins % 60}m ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

function showToast(msg, type = 'info') {
  const el = document.getElementById('toast-msg');
  if (!el) return;
  el.textContent = msg;
  el.className = `toast-msg show ${type}`;
  setTimeout(() => {
    el.className = 'toast-msg';
  }, 3500);
}

function copyToClipboard(text, event) {
  if (event) {
    event.stopPropagation();
    event.preventDefault();
  }
  if (!text) return;
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(() => {
      showToast('Copied to clipboard: ' + text.slice(0, 16) + '...', 'info');
    }).catch(() => fallbackCopy(text));
  } else {
    fallbackCopy(text);
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
    showToast('Copied to clipboard: ' + text.slice(0, 16) + '...', 'info');
  } catch (e) {
    console.error('Copy failed:', e);
  }
  document.body.removeChild(ta);
}

// --- TRU-EVOLVE Protocol Utilities ---
function isEvolveTx(data) {
  if (!data) return false;
  if (data.is_evolve || data.isEvolve) return true;
  const outputs = Array.isArray(data.outputs) ? data.outputs : [];
  return outputs.some(o => {
    const spk = (o.script_pubkey || '').toLowerCase();
    return spk.includes('5452555f45564f4c5645') || spk.includes('5452555f544f4b454e5f45564f4c5645');
  });
}

function parseEvolvePayload(spkHex) {
  if (!spkHex || typeof spkHex !== 'string') return null;
  const hex = spkHex.toLowerCase().replace(/^0x/, '');
  if (!hex.startsWith('6a')) return null;

  let idx = 2; // skip OP_RETURN (0x6a)
  const chunks = [];
  while (idx < hex.length) {
    const op = parseInt(hex.substr(idx, 2), 16);
    idx += 2;
    let len = 0;
    if (op <= 75) {
      len = op;
    } else if (op === 76) {
      len = parseInt(hex.substr(idx, 2), 16);
      idx += 2;
    } else {
      break;
    }
    if (idx + len * 2 > hex.length) break;
    const chunkHex = hex.substr(idx, len * 2);
    idx += len * 2;

    let str = '';
    let isAscii = true;
    for (let c = 0; c < chunkHex.length; c += 2) {
      const code = parseInt(chunkHex.substr(c, 2), 16);
      if (code >= 32 && code <= 126) {
        str += String.fromCharCode(code);
      } else {
        isAscii = false;
        break;
      }
    }
    chunks.push({ hex: chunkHex, str: isAscii ? str : null });
  }

  if (chunks.length > 0 && chunks[0].str && (chunks[0].str.includes('TRU_EVOLVE') || chunks[0].str.includes('TRU_TOKEN_EVOLVE'))) {
    return {
      protocol: chunks[0].str,
      tokenId: chunks[1] ? (chunks[1].str || chunks[1].hex) : '—',
      symbol: chunks[2] ? (chunks[2].str || 'NCFT') : 'NCFT',
      epoch: chunks[3] ? (chunks[3].str || '0') : '0',
      engine: chunks[4] ? (chunks[4].str || chunks[4].hex) : '—',
      mutation: chunks[5] ? (chunks[5].str || chunks[5].hex) : '—',
      prevHash: chunks[6] ? (chunks[6].str || chunks[6].hex) : '',
      newHash: chunks[7] ? (chunks[7].str || chunks[7].hex) : '',
    };
  }
  return null;
}

// --- Navigation & View Toggling ---

function showLiveFeedView() {
  const feedGrid = document.getElementById('explorer-feed-grid');
  const detailBox = document.getElementById('explorer-detail-box');
  if (feedGrid) feedGrid.style.display = 'grid';
  if (detailBox) {
    detailBox.style.display = 'none';
    detailBox.innerHTML = '';
  }
  if (window.location.search) {
    window.history.pushState(null, '', '/explorer');
  }
}

function showDetailView() {
  const feedGrid = document.getElementById('explorer-feed-grid');
  const detailBox = document.getElementById('explorer-detail-box');
  if (feedGrid) feedGrid.style.display = 'none';
  if (detailBox) detailBox.style.display = 'flex';
}

async function inspectTx(txid) {
  const cleanTxid = String(txid || '').trim();
  if (!cleanTxid) return;

  const feedGrid = document.getElementById('explorer-feed-grid');
  const detailBox = document.getElementById('explorer-detail-box');
  if (!detailBox) return;

  if (feedGrid) feedGrid.style.display = 'none';
  detailBox.style.display = 'flex';
  window.history.pushState(null, '', '/explorer?tx=' + encodeURIComponent(cleanTxid));

  detailBox.innerHTML = `
    <div class="tru-tx-loading">
      <div class="tru-tx-spinner"></div>
      <p style="font-weight: 600; color: #cbd5e1;">Loading Transaction Details from TRU Node...</p>
      <code style="font-family: var(--font-mono); font-size: 0.8rem; color: #8c9ba5;">${escapeHtml(cleanTxid)}</code>
    </div>
  `;

  try {
    const res = await fetch(EXPLORER_BASE + '/api/tx/' + cleanTxid);
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      throw new Error(err.error || `HTTP ${res.status}`);
    }
    const data = await res.json();
    renderTxDetailContent(data);
  } catch (err) {
    detailBox.innerHTML = `
      <div class="detail-nav-row">
        <button type="button" class="detail-back-btn" onclick="showLiveFeedView()">
          ← Back to Live Feeds
        </button>
      </div>
      <div class="tru-tx-notice-box" style="margin-top: 14px;">
        <div style="font-size: 32px;">⚠️</div>
        <h4 style="color: #f43f5e; font-size: 1.1rem; margin: 0;">Transaction Not Found</h4>
        <p style="font-size: 0.85rem; color: #8c9ba5; margin: 0;">${escapeHtml(err.message)}</p>
        <button type="button" class="mecha-mini-btn" style="height: 28px; padding: 0 14px; margin-top: 8px;" onclick="inspectTx('${cleanTxid}')">🔄 Retry</button>
      </div>
    `;
  }
}

function renderTxDetailContent(data) {
  const detailBox = document.getElementById('explorer-detail-box');
  if (!detailBox) return;

  const isConfirmed = !!data.is_confirmed;
  const confs = data.confirmations !== undefined ? data.confirmations : (isConfirmed ? 1 : 0);
  const blockHeight = data.block_height !== null && data.block_height !== undefined ? data.block_height : 'Pending in Mempool';
  const blockHash = data.block_hash || '—';

  const inputs = Array.isArray(data.inputs) ? data.inputs : [];
  const outputs = Array.isArray(data.outputs) ? data.outputs : [];
  const isQuantumTx = !!data.is_quantum
    || outputs.some(o => o.is_quantum || (o.address && o.address.startsWith('truq1:')) || (o.script_pubkey && (o.script_pubkey.includes('bb51') || o.script_pubkey.includes('bb'))))
    || inputs.some(i => i.is_quantum || (i.script_sig && i.script_sig.length >= 2000));
  const isEvolve = isEvolveTx(data);
  const evolveOutput = outputs.find(o => {
    const spk = (o.script_pubkey || '').toLowerCase();
    return spk.includes('5452555f45564f4c5645') || spk.includes('5452555f544f4b454e5f45564f4c5645');
  });
  const evolvePayload = evolveOutput ? parseEvolvePayload(evolveOutput.script_pubkey) : null;

  const inputsHtml = inputs.length > 0 ? inputs.map((inp, idx) => {
    const isCb = inp.is_coinbase || (inp.prev_txid && (inp.prev_txid.toLowerCase().includes('genesis') || inp.prev_txid.split('').every(c => c === '0')));
    const prevTx = inp.prev_txid || '0000000000000000000000000000000000000000000000000000000000000000';
    const shortPrev = prevTx.length > 18 ? `${prevTx.slice(0, 8)}...${prevTx.slice(-6)}` : prevTx;
    const isWotsSig = inp.is_quantum || (inp.script_sig && inp.script_sig.length >= 2000);
    const sigLenBytes = inp.script_sig ? Math.round(inp.script_sig.length / 2) : 0;
    return `
      <div class="tru-tx-io-item">
        <div class="tru-tx-target-group">
          ${isCb ? `<span style="color: #fbbf24; font-weight: 700;">🪙 Coinbase</span>` : `
            <span class="tru-tx-target-link" onclick="inspectTx('${escapeHtml(prevTx)}')" title="View Prevout: ${escapeHtml(prevTx)}">${escapeHtml(shortPrev)}</span>
            <button type="button" class="mecha-mini-btn" style="height: 18px; padding: 0 4px; font-size: 0.65rem;" onclick="copyToClipboard('${escapeHtml(prevTx)}', event)" title="Copy Prevout TXID">📋</button>
          `}
          ${isWotsSig ? `<span class="mini-tag-quantum" style="background: rgba(168, 85, 247, 0.2); color: #c084fc; border: 1px solid rgba(168, 85, 247, 0.4); border-radius: 4px; padding: 1px 6px; font-size: 0.65rem; font-weight: 700;">⚡ WOTS+ Signature (${sigLenBytes} B)</span>` : ''}
        </div>
        <div class="tru-tx-io-sub">
          ${isCb ? 'Mint' : `Output #${inp.prev_vout !== undefined ? inp.prev_vout : idx}`}
        </div>
      </div>
    `;
  }).join('') : `<div class="tru-tx-io-item" style="justify-content: center; color: #64748b;">No inputs recorded</div>`;

  const outputsHtml = outputs.length > 0 ? outputs.map(out => {
    const isQuantum = out.is_quantum || (out.address && out.address.startsWith('truq1:')) || (out.script_pubkey && out.script_pubkey.includes('bb51'));
    const isOutEvolveAnchor = (out.script_pubkey && ((out.script_pubkey.toLowerCase().includes('5452555f45564f4c5645') || out.script_pubkey.toLowerCase().includes('5452555f544f4b454e5f45564f4c5645')))) || (isEvolve && (out.amount_atoms === 0 || out.amount_tru === 0));
    const isCarrierReturn = isEvolve && !isOutEvolveAnchor && (out.amount_atoms > 0 || out.amount_tru > 0);

    const target = out.display_target || out.address || `${data.txid}:${out.vout}`;
    let shortTarget = target;
    if (isOutEvolveAnchor) {
      shortTarget = '🧬 TRU-EVOLVE Anchor Commitment';
    } else if (out.address && out.address.startsWith('truq1:')) {
      const parts = out.address.split(':');
      const scriptPart = parts[1] || '';
      const checksumPart = parts[2] || '';
      shortTarget = `truq1:${scriptPart.slice(0, 8)}...${scriptPart.slice(-8)}${checksumPart ? ':' + checksumPart.slice(0, 6) : ''}`;
    } else if (out.address) {
      shortTarget = out.address.length > 20 ? `${out.address.slice(0, 8)}...${out.address.slice(-6)}` : out.address;
    } else if (target.length > 20) {
      shortTarget = `${target.slice(0, 8)}...${target.slice(-6)}`;
    }
    const amt = typeof out.amount_tru === 'number' ? out.amount_tru : parseFloat(out.amount_tru || 0);
    const amtStr = amt.toFixed(amt > 0 && amt < 0.0001 ? 8 : (amt % 1 === 0 ? 0 : 7));
    const hasWotsVerify = out.script_pubkey && (out.script_pubkey.includes('bb51') || out.script_pubkey.includes('bb'));

    return `
      <div class="tru-tx-io-item">
        <div class="tru-tx-target-group">
          ${isOutEvolveAnchor ? `
            <span class="tru-tx-target-text" style="color: #c084fc; font-weight: 700;">🧬 TRU-EVOLVE Anchor</span>
            <span class="mini-tag-quantum" style="background: rgba(168, 85, 247, 0.2); color: #c084fc; border: 1px solid rgba(168, 85, 247, 0.4); border-radius: 4px; padding: 1px 6px; font-size: 0.65rem; font-weight: 700;">OP_RETURN</span>
          ` : (out.address ? `
            <span class="tru-tx-target-link" onclick="inspectAddress('${escapeHtml(out.address)}')" title="Filter Address: ${escapeHtml(out.address)}">${escapeHtml(shortTarget)}</span>
            <button type="button" class="mecha-mini-btn" style="height: 18px; padding: 0 4px; font-size: 0.65rem;" onclick="copyToClipboard('${escapeHtml(out.address)}', event)" title="Copy Address">📋</button>
          ` : `
            <span class="tru-tx-target-text" title="${escapeHtml(target)}">${escapeHtml(shortTarget)}</span>
            <button type="button" class="mecha-mini-btn" style="height: 18px; padding: 0 4px; font-size: 0.65rem;" onclick="copyToClipboard('${escapeHtml(target)}', event)" title="Copy Target">📋</button>
          `)}
          ${isCarrierReturn ? '<span class="mini-tag-vault" style="background: rgba(56, 189, 248, 0.15); color: #38bdf8; border: 1px solid rgba(56, 189, 248, 0.35); border-radius: 4px; padding: 1px 6px; font-size: 0.65rem; font-weight: 700;">↩ Self-Return / Carrier</span>' : ''}
          ${isQuantum ? '<span class="mini-tag-quantum" style="background: rgba(168, 85, 247, 0.2); color: #c084fc; border: 1px solid rgba(168, 85, 247, 0.4); border-radius: 4px; padding: 1px 6px; font-size: 0.65rem; font-weight: 700;">⚡ Quantum WOTS+</span>' : ''}
          ${hasWotsVerify ? '<span class="mini-tag-quantum" style="background: rgba(168, 85, 247, 0.12); color: #e879f9; border: 1px solid rgba(168, 85, 247, 0.3); border-radius: 4px; padding: 1px 5px; font-size: 0.65rem;" title="Opcode 0xbb: OP_CHECKWOTSVERIFY">OP_CHECKWOTSVERIFY</span>' : ''}
          ${out.is_htlc ? '<span class="mini-tag-htlc">HTLC Script</span>' : ''}
          ${out.is_vault ? '<span class="mini-tag-vault">Local Vault</span>' : ''}
        </div>
        <div class="tru-tx-io-amount">
          ${amtStr}<span class="tru-unit">TRU</span>
          ${isCarrierReturn ? '<div style="font-size: 0.65rem; color: #94a3b8; font-weight: 500; text-align: right;">Returned to Initiator</div>' : ''}
        </div>
      </div>
    `;
  }).join('') : `<div class="tru-tx-io-item" style="justify-content: center; color: #64748b;">No outputs</div>`;

  detailBox.innerHTML = `
    <div class="detail-nav-row">
      <button type="button" class="detail-back-btn" onclick="showLiveFeedView()">
        ← Back to Live Feeds
      </button>
      <div style="display: flex; gap: 8px;">
        ${isEvolve ? `
          <span class="badge-confs" style="background: rgba(168, 85, 247, 0.18); color: #c084fc; border-color: rgba(168, 85, 247, 0.45); font-weight: 700;">
            🧬 TRU-EVOLVE STATE MUTATION
          </span>
        ` : ''}
        ${isQuantumTx ? `
          <span class="badge-confs" style="background: rgba(168, 85, 247, 0.18); color: #c084fc; border-color: rgba(168, 85, 247, 0.45); font-weight: 700;">
            ⚡ QUANTUM WOTS+ SECURED
          </span>
        ` : ''}
        <span class="badge-confs" style="${isConfirmed ? '' : 'background: rgba(255, 184, 0, 0.12); color: var(--tru-gold); border-color: rgba(255, 184, 0, 0.3);'}">
          ${isConfirmed ? '● CONFIRMED' : '⏳ MEMPOOL'}
        </span>
        <span class="take-modal-pill-badge" style="color: #60a5fa;">
          ⛁ ${confs} ${confs === 1 ? 'Confirmation' : 'Confirmations'}
        </span>
      </div>
    </div>

    <!-- Banner Box -->
    <div class="tru-tx-banner-box">
      <div style="display: flex; justify-content: space-between; align-items: center;">
        <span class="tru-tx-banner-label">Transaction ID (TXID)</span>
        <button type="button" class="mecha-mini-btn" onclick="copyToClipboard('${escapeHtml(data.txid)}', event)">
          📋 Copy TXID
        </button>
      </div>
      <div class="tru-tx-hash-row">
        <span class="tru-tx-hash-val" title="${escapeHtml(data.txid)}">${escapeHtml(data.txid)}</span>
      </div>
    </div>

    ${isEvolve ? `
      <div class="tru-tx-notice-box" style="margin-top: 14px; background: rgba(168, 85, 247, 0.08); border: 1px solid rgba(168, 85, 247, 0.3); border-radius: 10px; padding: 14px 18px; text-align: left; display: flex; gap: 14px; align-items: flex-start;">
        <div style="font-size: 28px; line-height: 1;">🧬</div>
        <div style="flex: 1;">
          <div style="color: #c084fc; font-weight: 700; font-size: 0.95rem; margin-bottom: 4px; display: flex; align-items: center; gap: 8px;">
            TRU-EVOLVE Protocol Transaction (On-Chain State Mutation)
            <span style="font-size: 0.72rem; color: #a855f7; background: rgba(168, 85, 247, 0.2); padding: 2px 7px; border-radius: 4px; font-family: monospace;">CONTRACT STATE</span>
          </div>
          <div style="color: #cbd5e1; font-size: 0.8rem; line-height: 1.5;">
            In TRU-EVOLVE transactions, on-chain neural agent/token connectome state is cryptographically mutated and committed. No funds are transferred to third parties (<strong>External Transfer Volume: 0.00 TRU</strong>). The output amount is the initiator's token carrier / self-return balance returning back to their address.
          </div>
          ${evolvePayload ? `
            <div style="margin-top: 10px; padding-top: 10px; border-top: 1px dashed rgba(168, 85, 247, 0.25); display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 8px; font-size: 0.76rem;">
              <div><span style="color: #94a3b8;">Protocol:</span> <code style="color: #e879f9; font-weight: 700;">${escapeHtml(evolvePayload.protocol)}</code></div>
              <div><span style="color: #94a3b8;">Asset Symbol:</span> <code style="color: var(--accent-cyan); font-weight: 700;">${escapeHtml(evolvePayload.symbol)}</code></div>
              <div><span style="color: #94a3b8;">Token ID:</span> <code style="color: #f1f5f9;">${escapeHtml(evolvePayload.tokenId)}</code></div>
              <div><span style="color: #94a3b8;">Epoch:</span> <code style="color: var(--tru-gold); font-weight: 700;">#${escapeHtml(evolvePayload.epoch)}</code></div>
              <div><span style="color: #94a3b8;">Engine:</span> <code style="color: #f1f5f9;">${escapeHtml(evolvePayload.engine)}</code></div>
              <div><span style="color: #94a3b8;">Mutation:</span> <code style="color: #f1f5f9;">${escapeHtml(evolvePayload.mutation)}</code></div>
              ${evolvePayload.prevHash ? `<div style="grid-column: 1 / -1;"><span style="color: #94a3b8;">Prev Hash:</span> <code style="font-size: 0.7rem; color: #94a3b8; word-break: break-all;">${escapeHtml(evolvePayload.prevHash)}</code></div>` : ''}
              ${evolvePayload.newHash ? `<div style="grid-column: 1 / -1;"><span style="color: #94a3b8;">New State Hash:</span> <code style="font-size: 0.7rem; color: #a855f7; word-break: break-all;">${escapeHtml(evolvePayload.newHash)}</code></div>` : ''}
            </div>
          ` : ''}
        </div>
      </div>
    ` : ''}

    <!-- IO Grid -->
    <div class="tru-tx-io-grid">
      <div class="tru-tx-io-col">
        <div class="tru-tx-io-title inputs">
          <span>↓ Inputs (${inputs.length})</span>
        </div>
        <div class="tru-tx-io-list">
          ${inputsHtml}
        </div>
      </div>
      <div class="tru-tx-io-col">
        <div class="tru-tx-io-title outputs">
          <span>↑ Outputs (${outputs.length})</span>
        </div>
        <div class="tru-tx-io-list">
          ${outputsHtml}
        </div>
      </div>
    </div>

    <!-- Metadata Grid -->
    <div class="tru-tx-meta-box">
      <div class="tru-tx-meta-title">Block & Network Metadata</div>
      <div class="tru-tx-meta-grid">
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Transaction Type:</span>
          <span class="tru-tx-meta-val" style="color: ${isEvolve ? '#c084fc' : (data.is_coinbase ? 'var(--tru-gold)' : '#f1f5f9')}; font-weight: 700;">
            ${isEvolve ? '🧬 TRU-EVOLVE Contract Mutation' : (data.is_coinbase ? '🪙 Coinbase Mint' : 'Standard Transfer')}
          </span>
        </div>
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Transfer Volume:</span>
          <span class="tru-tx-meta-val" style="color: ${isEvolve ? '#c084fc' : 'var(--accent-cyan)'}; font-weight: 700;">
            ${isEvolve ? '0.00 TRU (Self-Return State Update)' : `${(outputs.reduce((acc, o) => acc + (o.amount_tru || 0), 0)).toFixed(4)} TRU`}
          </span>
        </div>
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Block Height:</span>
          <span class="tru-tx-meta-val highlight-gold">
            ${data.block_height !== null && data.block_height !== undefined ? `
              <a href="javascript:void(0)" onclick="inspectBlock('${data.block_height}')" style="color: var(--tru-gold); text-decoration: underline;">#${data.block_height}</a>
            ` : 'Unconfirmed'}
          </span>
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
            ${blockHash && blockHash !== '—' ? `
              <a href="javascript:void(0)" onclick="inspectBlock('${escapeHtml(blockHash)}')" style="color: #60a5fa; text-decoration: underline;">${escapeHtml(blockHash.slice(0, 14))}...${escapeHtml(blockHash.slice(-8))}</a>
              <button type="button" class="mecha-mini-btn" style="height: 18px; padding: 0 4px; font-size: 0.65rem; margin-left: 4px;" onclick="copyToClipboard('${escapeHtml(blockHash)}', event)">📋</button>
            ` : '—'}
          </span>
        </div>
        <div class="tru-tx-meta-row full-width">
          <span class="tru-tx-meta-label">Timestamp:</span>
          <span class="tru-tx-meta-val">${data.formatted_time || 'Recent'}${data.timestamp ? ` <span style="white-space: nowrap; color: #94a3b8; font-size: 0.74rem;">(<span class="live-time-ago" data-timestamp="${data.timestamp}">${timeAgo(data.timestamp)}</span>)</span>` : ''}</span>
        </div>
        ${isQuantumTx ? `
        <div class="tru-tx-meta-row full-width" style="background: rgba(168, 85, 247, 0.08); border: 1px solid rgba(168, 85, 247, 0.25); border-radius: 8px; padding: 12px 14px; margin-top: 6px;">
          <div style="display: flex; flex-direction: column; gap: 6px; width: 100%;">
            <div style="display: flex; justify-content: space-between; align-items: center;">
              <span style="color: #c084fc; font-weight: 700; font-size: 0.82rem; display: flex; align-items: center; gap: 6px;">
                ⚛️ Post-Quantum Protection Active (RFC 8391 Winternitz OTS)
              </span>
              <span style="font-size: 0.7rem; color: #a855f7; background: rgba(168, 85, 247, 0.2); padding: 2px 7px; border-radius: 4px; font-family: monospace;">OP_CHECKWOTSVERIFY (0xbb)</span>
            </div>
            <div style="font-size: 0.76rem; color: #cbd5e1; line-height: 1.45;">
              This transaction is protected by post-quantum Winternitz One-Time Signatures (WOTS+) executed directly on the TRU virtual machine via opcode <code>0xbb</code>. Keys and descriptors use the <code>truq1:</code> canonical format, resilient against Shor's algorithm on quantum computers.
            </div>
          </div>
        </div>
        ` : ''}
      </div>
    </div>
  `;
}

async function inspectBlock(query) {
  const cleanQ = String(query || '').trim();
  if (!cleanQ) return;

  const feedGrid = document.getElementById('explorer-feed-grid');
  const detailBox = document.getElementById('explorer-detail-box');
  if (!detailBox) return;

  if (feedGrid) feedGrid.style.display = 'none';
  detailBox.style.display = 'flex';
  window.history.pushState(null, '', '/explorer?block=' + encodeURIComponent(cleanQ));

  detailBox.innerHTML = `
    <div class="tru-tx-loading">
      <div class="tru-tx-spinner"></div>
      <p style="font-weight: 600; color: #cbd5e1;">Loading Block Telemetry from TRU Node...</p>
      <code style="font-family: var(--font-mono); font-size: 0.8rem; color: #8c9ba5;">${escapeHtml(cleanQ)}</code>
    </div>
  `;

  try {
    const res = await fetch(EXPLORER_BASE + '/api/block/' + cleanQ);
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      throw new Error(err.error || `HTTP ${res.status}`);
    }
    const data = await res.json();
    renderBlockDetailContent(data.block);
  } catch (err) {
    detailBox.innerHTML = `
      <div class="detail-nav-row">
        <button type="button" class="detail-back-btn" onclick="showLiveFeedView()">
          ← Back to Live Feeds
        </button>
      </div>
      <div class="tru-tx-notice-box" style="margin-top: 14px;">
        <div style="font-size: 32px;">⚠️</div>
        <h4 style="color: #f43f5e; font-size: 1.1rem; margin: 0;">Block Not Found</h4>
        <p style="font-size: 0.85rem; color: #8c9ba5; margin: 0;">${escapeHtml(err.message)}</p>
        <button type="button" class="mecha-mini-btn" style="height: 28px; padding: 0 14px; margin-top: 8px;" onclick="inspectBlock('${cleanQ}')">🔄 Retry</button>
      </div>
    `;
  }
}

function renderBlockDetailContent(b) {
  const detailBox = document.getElementById('explorer-detail-box');
  if (!detailBox || !b) return;

  const prevHeight = b.height > 0 ? b.height - 1 : null;
  const nextHeight = b.height < currentTipHeight ? b.height + 1 : null;

  const txsHtml = Array.isArray(b.txids) && b.txids.length > 0 ? b.txids.map((txid, idx) => `
    <div class="feed-tx-item" onclick="inspectTx('${escapeHtml(txid)}')">
      <div class="tx-item-left" style="max-width: 100%;">
        <div class="tx-id-row">
          <span style="color: var(--text-dim); font-size: 0.74rem; font-family: var(--font-mono);">#${idx + 1}</span>
          <span class="tx-id-text">${escapeHtml(txid)}</span>
          <button type="button" class="mecha-mini-btn" style="height: 18px; padding: 0 4px; font-size: 0.65rem;" onclick="copyToClipboard('${escapeHtml(txid)}', event)">📋</button>
        </div>
      </div>
    </div>
  `).join('') : `<div style="color: #64748b; padding: 12px; text-align: center;">No transactions found in this block.</div>`;

  detailBox.innerHTML = `
    <div class="detail-nav-row">
      <button type="button" class="detail-back-btn" onclick="showLiveFeedView()">
        ← Back to Live Feeds
      </button>
      <div style="display: flex; gap: 8px;">
        ${prevHeight !== null ? `
          <button type="button" class="detail-back-btn" onclick="inspectBlock('${prevHeight}')">
            ← Prev #${prevHeight}
          </button>
        ` : ''}
        ${nextHeight !== null ? `
          <button type="button" class="detail-back-btn" onclick="inspectBlock('${nextHeight}')">
            Next #${nextHeight} →
          </button>
        ` : ''}
      </div>
    </div>

    <!-- Block Header Card -->
    <div class="tru-tx-banner-box">
      <div style="display: flex; justify-content: space-between; align-items: center;">
        <span class="tru-tx-banner-label">Block #${b.height} Hash</span>
        <button type="button" class="mecha-mini-btn" onclick="copyToClipboard('${escapeHtml(b.hash)}', event)">
          📋 Copy Hash
        </button>
      </div>
      <div class="tru-tx-hash-row">
        <span class="tru-tx-hash-val" style="color: var(--tru-gold);">${escapeHtml(b.hash)}</span>
      </div>
      <div class="tru-tx-badges-row">
        <span class="badge-confs">● VERIFIED BLOCK</span>
        <span class="take-modal-pill-badge" style="color: var(--accent-cyan);">
          📦 ${b.tx_count} ${b.tx_count === 1 ? 'Transaction' : 'Transactions'}
        </span>
        <span class="take-modal-pill-badge" style="color: #cbd5e1;">
          ⚖️ ${(b.total_amount_tru || 0).toFixed(4)} TRU Output
        </span>
      </div>
    </div>

    <!-- Metadata Grid -->
    <div class="tru-tx-meta-box">
      <div class="tru-tx-meta-title">Block Header Data & Proof of Work</div>
      <div class="tru-tx-meta-grid">
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Block Height:</span>
          <span class="tru-tx-meta-val highlight-gold">#${b.height}</span>
        </div>
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Transactions Count:</span>
          <span class="tru-tx-meta-val">${b.tx_count}</span>
        </div>
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Nonce:</span>
          <span class="tru-tx-meta-val">${b.nonce}</span>
        </div>
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Target Bits:</span>
          <span class="tru-tx-meta-val">0x${(b.bits || 0).toString(16)}</span>
        </div>
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Block Size:</span>
          <span class="tru-tx-meta-val">${b.size_bytes} bytes</span>
        </div>
        <div class="tru-tx-meta-row">
          <span class="tru-tx-meta-label">Version:</span>
          <span class="tru-tx-meta-val">${b.version}</span>
        </div>
        <div class="tru-tx-meta-row full-width">
          <span class="tru-tx-meta-label">Previous Block Hash:</span>
          <span class="tru-tx-meta-val">
            ${b.prev_hash ? `
              <a href="javascript:void(0)" onclick="inspectBlock('${escapeHtml(b.prev_hash)}')" style="color: #60a5fa; text-decoration: underline;">${escapeHtml(b.prev_hash)}</a>
            ` : 'Genesis'}
          </span>
        </div>
        <div class="tru-tx-meta-row full-width">
          <span class="tru-tx-meta-label">Merkle Root:</span>
          <span class="tru-tx-meta-val" title="${escapeHtml(b.merkle_root)}">${escapeHtml(b.merkle_root)}</span>
        </div>
        <div class="tru-tx-meta-row full-width">
          <span class="tru-tx-meta-label">Timestamp:</span>
          <span class="tru-tx-meta-val">${b.formatted_time || 'Recent'}${b.timestamp ? ` <span style="white-space: nowrap; color: #94a3b8; font-size: 0.74rem;">(<span class="live-time-ago" data-timestamp="${b.timestamp}">${timeAgo(b.timestamp)}</span>)</span>` : ''}</span>
        </div>
      </div>
    </div>

    <!-- Included Transactions List -->
    <div class="tru-tx-meta-box">
      <div class="tru-tx-meta-title" style="display: flex; justify-content: space-between; align-items: center;">
        <span>Included Transactions (${b.tx_count})</span>
      </div>
      <div style="display: flex; flex-direction: column; gap: 8px; margin-top: 10px;">
        ${txsHtml}
      </div>
    </div>
  `;
}

let currentAddressTxsLimit = 30;
let currentAddressData = null;

function loadMoreAddressTransactions() {
  currentAddressTxsLimit += 30;
  if (currentAddressData) {
    const txs = Array.isArray(currentAddressData.txs) ? currentAddressData.txs : [];
    const addr = currentAddressData.address || '';
    renderAddressTransactionsList(txs, addr);
  }
}
window.loadMoreAddressTransactions = loadMoreAddressTransactions;

function renderAddressTransactionsList(txs, addr) {
  const container = document.getElementById('address-txs-list');
  const titleEl = document.getElementById('address-txs-title');
  if (!container) return;

  const total = txs.length;
  const displayed = txs.slice(0, currentAddressTxsLimit);

  if (titleEl) {
    if (total > displayed.length) {
      titleEl.textContent = `Observed Transactions (${displayed.length} of ${total})`;
    } else {
      titleEl.textContent = `Observed Transactions (${total})`;
    }
  }

  if (total === 0) {
    container.innerHTML = `<div style="color: #64748b; padding: 20px; text-align: center;">No transactions found on-chain for this address.</div>`;
    return;
  }

  const itemsHtml = displayed.map((tx, idx) => {
    let receivedAtoms = 0;
    if (Array.isArray(tx.outputs)) {
      for (const out of tx.outputs) {
        if (out.address === addr) {
          receivedAtoms += out.amount_atoms || 0;
        }
      }
    }
    const receivedTru = receivedAtoms / 100000000;
    const flowBadge = receivedAtoms > 0
      ? `<span class="badge-confs" style="background: rgba(0, 255, 102, 0.12); color: #00ff66; border-color: rgba(0, 255, 102, 0.35); font-weight: 700;">+${receivedTru.toFixed(4)} TRU</span>`
      : '';

    return `
      <div class="feed-tx-item" onclick="inspectTx('${escapeHtml(tx.txid)}')">
        <div class="tx-item-left">
          <div class="tx-id-row">
            <span style="color: var(--text-dim); font-size: 0.74rem;">#${idx + 1}</span>
            <span class="tx-id-text">${escapeHtml(tx.txid)}</span>
            <button type="button" class="mecha-mini-btn" style="height: 18px; padding: 0 4px; font-size: 0.65rem;" onclick="copyToClipboard('${escapeHtml(tx.txid)}', event)">📋</button>
          </div>
          <div class="tx-badges-row">
            <span class="badge-confs">Block #${tx.block_height || 'Pending'}</span>
            ${flowBadge}
            <span style="font-size: 0.7rem; color: #8c9ba5;" class="live-time-ago" data-timestamp="${tx.timestamp}">${timeAgo(tx.timestamp)}</span>
          </div>
        </div>
        <div class="tx-item-right">
          <span class="block-txs-badge" style="cursor: pointer;">Inspect ↗</span>
        </div>
      </div>
    `;
  }).join('');

  let loadMoreHtml = '';
  if (total > currentAddressTxsLimit) {
    loadMoreHtml = `
      <div style="text-align: center; padding: 12px 0 4px 0;">
        <button type="button" class="mecha-mini-btn" style="height: 36px; padding: 0 20px; font-size: 0.82rem; font-weight: 700; width: 100%; justify-content: center;" onclick="loadMoreAddressTransactions()">
          ⬇ Load More Transactions (+30)
        </button>
      </div>
    `;
  } else if (total > 30) {
    loadMoreHtml = `
      <div style="text-align: center; color: var(--text-dim); padding: 12px 0 4px 0; font-size: 0.76rem; font-family: var(--font-mono);">
        ✓ All ${total} transactions displayed
      </div>
    `;
  }

  container.innerHTML = itemsHtml + loadMoreHtml;
}

async function inspectAddress(addr) {
  const cleanA = String(addr || '').trim();
  if (!cleanA) return;

  currentAddressTxsLimit = 30;
  currentAddressData = null;

  const feedGrid = document.getElementById('explorer-feed-grid');
  const detailBox = document.getElementById('explorer-detail-box');
  if (!detailBox) return;

  if (feedGrid) feedGrid.style.display = 'none';
  detailBox.style.display = 'flex';
  window.history.pushState(null, '', '/explorer?address=' + encodeURIComponent(cleanA));

  detailBox.innerHTML = `
    <div class="tru-tx-loading">
      <div class="tru-tx-spinner"></div>
      <p style="font-weight: 600; color: #cbd5e1;">Querying Address History from TRU Ledger...</p>
      <code style="font-family: var(--font-mono); font-size: 0.8rem; color: #8c9ba5;">${escapeHtml(cleanA)}</code>
    </div>
  `;

  try {
    const res = await fetch(EXPLORER_BASE + '/api/explorer/search?q=' + encodeURIComponent(cleanA));
    const data = await res.json();
    if (!data.found || data.target_type !== 'address') {
      throw new Error(data.error || 'Address not observed in current scanned block window.');
    }
    renderAddressDetailContent(data);
  } catch (err) {
    detailBox.innerHTML = `
      <div class="detail-nav-row">
        <button type="button" class="detail-back-btn" onclick="showLiveFeedView()">
          ← Back to Live Feeds
        </button>
      </div>
      <div class="tru-tx-notice-box" style="margin-top: 14px;">
        <div style="font-size: 32px;">ℹ️</div>
        <h4 style="color: #38bdf8; font-size: 1.1rem; margin: 0;">Address Notice</h4>
        <p style="font-size: 0.85rem; color: #8c9ba5; margin: 0;">${escapeHtml(err.message)}</p>
        <button type="button" class="mecha-mini-btn" style="height: 28px; padding: 0 14px; margin-top: 8px;" onclick="inspectAddress('${cleanA}')">🔄 Refresh</button>
      </div>
    `;
  }
}

function renderAddressDetailContent(data) {
  const detailBox = document.getElementById('explorer-detail-box');
  if (!detailBox) return;

  currentAddressData = data;
  const addr = data.address || '';
  const txs = Array.isArray(data.txs) ? data.txs : [];

  detailBox.innerHTML = `
    <div class="detail-nav-row">
      <button type="button" class="detail-back-btn" onclick="showLiveFeedView()">
        ← Back to Live Feeds
      </button>
    </div>

    <!-- Address Card -->
    <div class="tru-tx-banner-box">
      <div style="display: flex; justify-content: space-between; align-items: center;">
        <span class="tru-tx-banner-label">TRU Wallet Address</span>
        <button type="button" class="mecha-mini-btn" onclick="copyToClipboard('${escapeHtml(addr)}', event)">
          📋 Copy Address
        </button>
      </div>
      <div class="tru-tx-hash-row">
        <span class="tru-tx-hash-val" style="color: var(--accent-cyan); font-size: clamp(0.74rem, 3.6vw, 0.98rem); letter-spacing: -0.2px;">${escapeHtml(addr)}</span>
      </div>
      <div class="tru-tx-badges-row">
        <span class="badge-confs">● ACTIVE</span>
        <span class="badge-confs" style="background: rgba(0, 240, 255, 0.15); color: #00f0ff; border-color: rgba(0, 240, 255, 0.4); font-size: 0.72rem; font-weight: 800;">
          💰 ${(data.balance_tru || 0).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 4 })} TRU
        </span>
        <span class="take-modal-pill-badge" style="color: var(--tru-gold); font-size: 0.70rem;">
          ⚡ ${txs.length} TXs
        </span>
      </div>
    </div>

    <!-- Financial Telemetry Grid -->
    <div class="address-stats-grid">
      <div class="explorer-stat-card address-stat-hero">
        <div class="stat-card-label">
          <span style="color: var(--accent-cyan); font-weight: 700;">Confirmed Balance</span>
          <span>💰</span>
        </div>
        <div class="stat-card-val hero-val" style="color: #00f0ff;">
          ${(data.balance_tru || 0).toLocaleString('en-US', { minimumFractionDigits: 4, maximumFractionDigits: 8 })} <span class="stat-unit">TRU</span>
        </div>
        <div class="stat-card-sub" style="color: #94a3b8;">
          ${(data.utxo_count || 0).toLocaleString()} Unspent UTXO outputs
        </div>
      </div>

      <div class="explorer-stat-card">
        <div class="stat-card-label">
          <span>Total Received</span>
          <span>📥</span>
        </div>
        <div class="stat-card-val" style="color: var(--accent-green); font-size: clamp(0.92rem, 3.6vw, 1.25rem);">
          +${(data.total_received_tru || 0).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })} <span class="stat-unit">TRU</span>
        </div>
        <div class="stat-card-sub">Cumulative inflow</div>
      </div>

      <div class="explorer-stat-card">
        <div class="stat-card-label">
          <span>Total Sent / Spent</span>
          <span>📤</span>
        </div>
        <div class="stat-card-val" style="color: #f43f5e; font-size: clamp(0.92rem, 3.6vw, 1.25rem);">
          -${(data.total_sent_tru || 0).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })} <span class="stat-unit">TRU</span>
        </div>
        <div class="stat-card-sub">Cumulative outflow</div>
      </div>
    </div>

    <!-- Transactions List -->
    <div class="tru-tx-meta-box" style="margin-top: 4px;">
      <div class="tru-tx-meta-title" id="address-txs-title">Observed Transactions</div>
      <div id="address-txs-list" style="display: flex; flex-direction: column; gap: 8px; margin-top: 10px;">
        <!-- Injected via renderAddressTransactionsList -->
      </div>
    </div>
  `;

  renderAddressTransactionsList(txs, addr);
}

// --- Mempool Inspector ---

async function inspectMempool(preloadedData) {
  showDetailView();
  const detailBox = document.getElementById('explorer-detail-box');
  if (!detailBox) return;

  history.replaceState(null, '', '/explorer?view=mempool');

  detailBox.innerHTML = `
    <div class="tru-tx-loading">
      <div class="tru-tx-spinner"></div>
      <p style="font-weight: 600; color: #cbd5e1;">Loading Mempool...</p>
    </div>
  `;

  try {
    let data = preloadedData;
    if (!data) {
      const res = await fetch(EXPLORER_BASE + '/api/mempool');
      if (!res.ok) throw new Error('HTTP ' + res.status);
      data = await res.json();
    }
    renderMempoolDetailContent(data);
  } catch (err) {
    detailBox.innerHTML = `
      <div class="detail-nav-row">
        <button type="button" class="detail-back-btn" onclick="showLiveFeedView()">
          ← Back to Live Feeds
        </button>
      </div>
      <div class="tru-tx-notice-box" style="margin-top: 14px;">
        <div style="font-size: 32px;">⚠️</div>
        <h4 style="color: #f43f5e; font-size: 1.1rem; margin: 0;">Failed to Load Mempool</h4>
        <p style="font-size: 0.85rem; color: #8c9ba5; margin: 0;">${escapeHtml(err.message)}</p>
        <button type="button" class="mecha-mini-btn" style="height: 28px; padding: 0 14px; margin-top: 8px;" onclick="inspectMempool()">🔄 Retry</button>
      </div>
    `;
  }
}

function renderMempoolDetailContent(data) {
  const detailBox = document.getElementById('explorer-detail-box');
  if (!detailBox) return;

  const rawTxs = Array.isArray(data.transactions) ? data.transactions : (Array.isArray(data) ? data : []);
  const count = data.count !== undefined ? data.count : rawTxs.length;
  let totalTru = 0;
  for (const tx of rawTxs) {
    const isEv = isEvolveTx(tx);
    if (!isEv) {
      totalTru += tx.total_out_tru || (tx.total_out_atoms ? tx.total_out_atoms / 100000000 : 0);
    }
  }

  const txsListHtml = rawTxs.length > 0 ? rawTxs.map((tx, idx) => {
    const txid = tx.txid || '';
    const isEv = isEvolveTx(tx);
    const amount = isEv ? 0.0 : (tx.total_out_tru !== undefined ? tx.total_out_tru : (tx.total_out_atoms || 0) / 100000000);
    const timeText = tx.first_seen_sec ? timeAgo(tx.first_seen_sec) : 'Just now';
    const isHtlc = !!tx.is_htlc;
    const isVault = !!tx.is_vault_related;

    return `
      <div class="feed-tx-item" onclick="inspectTx('${escapeHtml(txid)}')" style="cursor: pointer; padding: 12px 14px;">
        <div class="tx-item-left">
          <div class="tx-id-row" style="gap: 8px;">
            <span class="badge-confs" style="background: rgba(255, 184, 0, 0.15); color: #f59e0b; border-color: rgba(255, 184, 0, 0.4); font-size: 0.7rem; font-weight: 700;">⏳ PENDING</span>
            <span class="tx-id-text" style="font-size: 0.82rem;">${escapeHtml(txid.slice(0, 16))}...${escapeHtml(txid.slice(-10))}</span>
            <button type="button" class="mecha-mini-btn" style="height: 18px; padding: 0 4px; font-size: 0.65rem;" onclick="copyToClipboard('${escapeHtml(txid)}', event)">📋</button>
          </div>
          <div class="tx-badges-row" style="margin-top: 6px;">
            ${isEv ? '<span class="mini-tag-evolve" style="background: rgba(168, 85, 247, 0.2); color: #c084fc; border: 1px solid rgba(168, 85, 247, 0.4); border-radius: 4px; padding: 1px 6px; font-size: 0.68rem; font-weight: 700;">🧬 TRU-EVOLVE</span>' : ''}
            ${isHtlc ? '<span class="mini-tag-htlc" style="font-weight: 700;">HTLC SWAP</span>' : ''}
            ${isVault ? '<span class="mini-tag-vault" style="font-weight: 700;">VAULT</span>' : ''}
            <span style="font-size: 0.72rem; color: #94a3b8;">${tx.inputs_count || (tx.inputs ? tx.inputs.length : 1)} in / ${tx.outputs_count || (tx.outputs ? tx.outputs.length : 1)} out</span>
            <span style="font-size: 0.72rem; color: #64748b;" class="live-time-ago" data-timestamp="${tx.first_seen_sec || 0}">First seen: ${timeText}</span>
          </div>
        </div>
        <div class="tx-item-right" style="text-align: right;">
          <div class="tx-amount-val" style="font-size: 1rem; color: ${isEv ? '#c084fc' : '#38bdf8'};">
            ${isEv ? '0.0000 <span class="unit" style="font-size: 0.75rem; color: var(--tru-gold);">TRU</span> <span style="font-size: 0.7rem; color: #94a3b8; font-weight: 500;">(State Update)</span>' : `${amount.toFixed(4)} <span class="unit" style="font-size: 0.75rem;">TRU</span>`}
          </div>
          <span style="font-size: 0.75rem; color: var(--accent-cyan); font-weight: 700;">Inspect Transaction ↗</span>
        </div>
      </div>
    `;
  }).join('') : `
    <div style="text-align: center; padding: 45px 20px; background: rgba(15, 23, 42, 0.4); border-radius: 12px; border: 1px dashed rgba(255, 255, 255, 0.08);">
      <div style="font-size: 38px; margin-bottom: 12px;">⚡</div>
      <h4 style="color: #f8fafc; font-size: 1.1rem; margin: 0 0 8px 0; font-weight: 700;">Mempool is Empty</h4>
      <p style="color: #94a3b8; font-size: 0.85rem; max-width: 540px; margin: 0 auto; line-height: 1.5;">
        Currently there are no unconfirmed transactions awaiting mining.
      </p>
    </div>
  `;

  detailBox.innerHTML = `
    <!-- Nav Bar -->
    <div class="detail-nav-row" style="display: flex; justify-content: space-between; align-items: center;">
      <button type="button" class="detail-back-btn" onclick="showLiveFeedView()">
        ← Back to Live Feeds
      </button>
      <div style="display: flex; gap: 8px;">
        <button type="button" class="mecha-mini-btn" style="height: 30px; padding: 0 14px; font-weight: 700;" onclick="inspectMempool()">
          🔄 Refresh Mempool
        </button>
      </div>
    </div>

    <!-- Telemetry Cards -->
    <div class="explorer-stat-cards-grid" style="grid-template-columns: repeat(2, 1fr); margin-top: 14px; margin-bottom: 14px;">
      <div class="explorer-stat-card">
        <div class="stat-card-label">
          <span>Unconfirmed Transactions</span>
          <span>⚡</span>
        </div>
        <div class="stat-card-val" style="color: #f59e0b;">${count} TXs</div>
        <div class="stat-card-sub">Awaiting block inclusion</div>
      </div>

      <div class="explorer-stat-card">
        <div class="stat-card-label">
          <span>Total Pending Value</span>
          <span>💰</span>
        </div>
        <div class="stat-card-val" style="color: #38bdf8;">${totalTru.toFixed(4)} TRU</div>
        <div class="stat-card-sub">Aggregate transaction outputs</div>
      </div>
    </div>

    <!-- Transactions List Container -->
    <div class="tru-tx-meta-box">
      <div class="tru-tx-meta-title" style="display: flex; justify-content: space-between; align-items: center;">
        <span>Mempool Transactions (${count})</span>
        <span style="font-size: 0.75rem; color: #94a3b8; font-weight: 500;">Sorted by Arrival Time (Newest First)</span>
      </div>
      <div style="display: flex; flex-direction: column; gap: 8px; margin-top: 12px;">
        ${txsListHtml}
      </div>
    </div>
  `;
}

// --- Search Handler ---

async function handleExplorerSearch(event) {
  if (event) event.preventDefault();
  const input = document.getElementById('explorer-search-input');
  if (!input) return;
  const q = input.value.trim();
  if (!q) return;

  // Instant direct mempool routing
  if (q.toLowerCase() === 'mempool') {
    inspectMempool();
    return;
  }

  // Instant direct height routing if purely numbers
  if (/^\d+$/.test(q)) {
    inspectBlock(q);
    return;
  }

  showToast('Searching ledger for ' + q.slice(0, 16) + '...', 'info');

  try {
    const res = await fetch(EXPLORER_BASE + '/api/explorer/search?q=' + encodeURIComponent(q));
    const data = await res.json();
    if (data.found) {
      if (data.target_type === 'block') {
        inspectBlock(data.block.height || data.block.hash);
      } else if (data.target_type === 'tx') {
        inspectTx(data.tx.txid);
      } else if (data.target_type === 'address') {
        inspectAddress(data.address);
      } else if (data.target_type === 'mempool') {
        inspectMempool(data);
      }
    } else {
      showToast(data.error || 'No matching block, transaction, or address found.', 'error');
    }
  } catch (err) {
    showToast('Search query error: ' + err.message, 'error');
  }
}

// --- Real-Time Feeds Renderer ---

let currentBlocksLimit = 30;
let currentTxsLimit = 30;

function loadMoreBlocks() {
  currentBlocksLimit += 30;
  pollExplorerTelemetry();
}
window.loadMoreBlocks = loadMoreBlocks;

function loadMoreTransactions() {
  currentTxsLimit += 30;
  if (currentBlocksLimit < currentTxsLimit) {
    currentBlocksLimit = currentTxsLimit;
  }
  pollExplorerTelemetry();
}
window.loadMoreTransactions = loadMoreTransactions;

function renderBlocksFeed(blocks) {
  const container = document.getElementById('feed-blocks-list');
  if (!container || !Array.isArray(blocks)) return;

  const displayedBlocks = blocks.slice(0, currentBlocksLimit);

  if (displayedBlocks.length === 0) {
    container.innerHTML = `<div style="text-align: center; color: var(--text-dim); padding: 30px;">No blocks in cache yet. Syncing...</div>`;
    return;
  }

  const isNewTip = lastSeenBlockHash && displayedBlocks[0] && displayedBlocks[0].hash !== lastSeenBlockHash;
  if (displayedBlocks[0]) {
    lastSeenBlockHash = displayedBlocks[0].hash;
  }

  const itemsHtml = displayedBlocks.map((b, idx) => {
    const isNew = isNewTip && idx === 0;
    return `
      <div class="feed-block-item ${isNew ? 'just-added' : ''}" onclick="inspectBlock('${b.height}')" title="Click to inspect Block #${b.height}">
        <div class="block-item-left">
          <div class="block-height-pill">#${b.height}</div>
          <div class="block-meta-col">
            <span class="block-hash-short" title="${escapeHtml(b.hash)}">${escapeHtml(b.hash.slice(0, 10))}...${escapeHtml(b.hash.slice(-8))}</span>
            <span class="block-time-ago live-time-ago" data-timestamp="${b.timestamp}">${timeAgo(b.timestamp)}</span>
          </div>
        </div>
        <div class="block-item-right">
          <span class="block-txs-badge">${b.tx_count} ${b.tx_count === 1 ? 'tx' : 'txs'}</span>
          <span class="block-vol-text">${(b.total_amount_tru || 0).toFixed(2)} TRU</span>
        </div>
      </div>
    `;
  }).join('');

  const loadMoreBtn = `
    <div style="text-align: center; padding: 10px 0 4px 0;">
      <button type="button" class="mecha-mini-btn" style="height: 32px; padding: 0 16px; font-size: 0.8rem; font-weight: 700; width: 100%; justify-content: center;" onclick="loadMoreBlocks()">
        ⬇ Load More Blocks (+30)
      </button>
    </div>
  `;

  container.innerHTML = itemsHtml + loadMoreBtn;
}

function renderTransactionsFeed(mempoolTxs, recentBlocks) {
  const container = document.getElementById('feed-txs-list');
  if (!container) return;

  let allTxs = [];

  // 1. Mempool Txs first
  if (Array.isArray(mempoolTxs)) {
    for (const m of mempoolTxs) {
      const isEv = isEvolveTx(m);
      allTxs.push({
        txid: m.txid,
        isMempool: true,
        height: null,
        timestamp: m.first_seen_sec,
        amountTru: isEv ? 0.0 : (m.total_out_tru || 0),
        isEvolve: isEv,
        isHtlc: m.is_htlc,
        isVault: m.is_vault_related,
      });
    }
  }

  // 2. Mined user transactions from recent blocks (exclude technical coinbase mints)
  if (Array.isArray(recentBlocks)) {
    for (const b of recentBlocks) {
      if (Array.isArray(b.tx_summaries) && b.tx_summaries.length > 0) {
        for (const s of b.tx_summaries) {
          if (s.is_coinbase) continue; // exclude coinbase mint
          allTxs.push({
            txid: s.txid,
            isMempool: false,
            height: b.height,
            timestamp: b.timestamp,
            amountTru: s.is_evolve ? 0.0 : (s.amount_tru || 0),
            isEvolve: s.is_evolve,
            isHtlc: s.is_htlc,
            isVault: false,
          });
          if (allTxs.length >= currentTxsLimit + 50) break;
        }
      } else if (Array.isArray(b.txids) && b.txids.length > 1) {
        // Fallback for older blocks
        const userTxs = b.txids.slice(1);
        const userAmount = Math.max(0, (b.total_amount_tru || 0) - 50.0);
        const amountPerTx = userTxs.length > 0 ? (userAmount / userTxs.length) : userAmount;
        for (const txid of userTxs) {
          allTxs.push({
            txid: txid,
            isMempool: false,
            height: b.height,
            timestamp: b.timestamp,
            amountTru: amountPerTx,
            isEvolve: false,
            isHtlc: false,
            isVault: false,
          });
          if (allTxs.length >= currentTxsLimit + 50) break;
        }
      }
      if (allTxs.length >= currentTxsLimit + 50) break;
    }
  }

  const displayedTxs = allTxs.slice(0, currentTxsLimit);

  if (displayedTxs.length === 0) {
    container.innerHTML = `<div style="text-align: center; color: var(--text-dim); padding: 30px;">No recent user transactions</div>`;
    return;
  }

  const itemsHtml = displayedTxs.map(t => {
    return `
      <div class="feed-tx-item" onclick="inspectTx('${escapeHtml(t.txid)}')" title="Click to inspect Transaction ${escapeHtml(t.txid)}">
        <div class="tx-item-left">
          <div class="tx-id-row">
            <span class="tx-id-text">${escapeHtml(t.txid.slice(0, 10))}...${escapeHtml(t.txid.slice(-8))}</span>
            <button type="button" class="mecha-mini-btn" style="height: 18px; padding: 0 4px; font-size: 0.65rem;" onclick="copyToClipboard('${escapeHtml(t.txid)}', event)">📋</button>
          </div>
          <div class="tx-badges-row">
            ${t.isMempool ? `
              <span class="badge-confs" style="background: rgba(255, 184, 0, 0.12); color: var(--tru-gold); border-color: rgba(255, 184, 0, 0.3); font-size: 0.65rem; padding: 1px 5px;">⏳ MEMPOOL</span>
            ` : `
              <span class="badge-confs" style="font-size: 0.65rem; padding: 1px 5px;">#${t.height}</span>
            `}
            ${t.isEvolve ? '<span class="mini-tag-evolve" style="background: rgba(168, 85, 247, 0.18); color: #c084fc; border: 1px solid rgba(168, 85, 247, 0.4); border-radius: 4px; padding: 1px 6px; font-size: 0.65rem; font-weight: 700;">🧬 TRU-EVOLVE</span>' : ''}
            ${t.isHtlc ? '<span class="mini-tag-htlc">HTLC</span>' : ''}
            ${t.isVault ? '<span class="mini-tag-vault">Vault</span>' : ''}
            <span style="font-size: 0.7rem; color: #64748b;" class="live-time-ago" data-timestamp="${t.timestamp}">${timeAgo(t.timestamp)}</span>
          </div>
        </div>
        <div class="tx-item-right">
          <div class="tx-amount-val">
            ${t.isEvolve ? `
              <span style="color: #c084fc; font-weight: 700;">0.00 <span class="unit" style="color: var(--tru-gold);">TRU</span></span>
              <span style="font-size: 0.68rem; color: #94a3b8; font-weight: 500; margin-left: 2px;">(State Update)</span>
            ` : `
              ${t.amountTru.toFixed(2)} <span class="unit">TRU</span>
            `}
          </div>
          <span style="font-size: 0.7rem; color: var(--accent-cyan); font-weight: 700;">Details ↗</span>
        </div>
      </div>
    `;
  }).join('');

  const loadMoreBtn = `
    <div style="text-align: center; padding: 10px 0 4px 0;">
      <button type="button" class="mecha-mini-btn" style="height: 32px; padding: 0 16px; font-size: 0.8rem; font-weight: 700; width: 100%; justify-content: center;" onclick="loadMoreTransactions()">
        ⬇ Load More Transactions (+30)
      </button>
    </div>
  `;

  container.innerHTML = itemsHtml + loadMoreBtn;
}

// --- Telemetry Polling Loop ---

async function pollExplorerTelemetry() {
  try {
    const fetchLimit = Math.max(currentBlocksLimit, currentTxsLimit);
    const [statsRes, blocksRes, mempoolRes] = await Promise.all([
      fetch(EXPLORER_BASE + '/api/explorer/stats').then(r => r.ok ? r.json() : null),
      fetch(`${EXPLORER_BASE}/api/blocks?limit=${fetchLimit}`).then(r => r.ok ? r.json() : null),
      fetch(EXPLORER_BASE + '/api/mempool').then(r => r.ok ? r.json() : null),
    ]);

    if (statsRes) {
      currentTipHeight = statsRes.tip_height || 0;
      lastBlockTimestamp = statsRes.last_block_time || 0;

      const tipEl = document.getElementById('stat-tip-height');
      const tipHashEl = document.getElementById('stat-tip-hash');
      const hdrStatusEl = document.getElementById('header-chain-status');
      const peersEl = document.getElementById('stat-peers-count');
      const hdrPeersEl = document.getElementById('header-peers-badge');
      const mempoolCountEl = document.getElementById('stat-mempool-count');
      const mempoolVolEl = document.getElementById('stat-mempool-vol');
      const hdrMempoolEl = document.getElementById('header-mempool-count');

      if (tipEl) tipEl.textContent = '#' + currentTipHeight;
      if (hdrStatusEl) hdrStatusEl.textContent = '#' + currentTipHeight;
      if (tipHashEl && statsRes.tip_hash) {
        tipHashEl.textContent = statsRes.tip_hash.slice(0, 16) + '...';
        tipHashEl.title = statsRes.tip_hash;
      }
      if (peersEl) peersEl.textContent = (statsRes.peers || 0) + ' Peers';
      if (hdrPeersEl) hdrPeersEl.textContent = (statsRes.peers || 0) + ' peers';
      if (mempoolCountEl) mempoolCountEl.textContent = (statsRes.mempool_count || 0) + ' TXs';
      if (hdrMempoolEl) hdrMempoolEl.textContent = (statsRes.mempool_count || 0) + ' txs';
      if (mempoolVolEl) mempoolVolEl.textContent = ((statsRes.mempool_tru || 0).toFixed(2)) + ' TRU pending';
    }

    if (blocksRes && Array.isArray(blocksRes)) {
      recentBlocksCache = blocksRes;
      renderBlocksFeed(blocksRes);
    }

    const mempoolList = (mempoolRes && Array.isArray(mempoolRes.transactions))
      ? mempoolRes.transactions
      : (Array.isArray(mempoolRes) ? mempoolRes : []);
    mempoolTxsCache = mempoolList;
    renderTransactionsFeed(mempoolTxsCache, recentBlocksCache);
  } catch (err) {
    console.warn('[EXPLORER] Telemetry poll failed:', err);
  }
}

// Tick time ago counters every second across all views
setInterval(() => {
  const blockTimeEl = document.getElementById('stat-block-time');
  const blockUtcEl = document.getElementById('stat-block-utc');
  if (blockTimeEl && lastBlockTimestamp > 0) {
    blockTimeEl.textContent = timeAgo(lastBlockTimestamp);
  }
  if (blockUtcEl && lastBlockTimestamp > 0) {
    const dt = new Date(lastBlockTimestamp * 1000);
    blockUtcEl.textContent = dt.toUTCString().replace('GMT', 'UTC');
  }

  // Dynamically tick every active timeAgo span on the screen
  document.querySelectorAll('.live-time-ago[data-timestamp]').forEach(el => {
    const ts = parseInt(el.getAttribute('data-timestamp'), 10);
    if (!isNaN(ts) && ts > 0) {
      el.textContent = timeAgo(ts);
    }
  });
}, 1000);

// Initial Load & Route Param Checking
window.addEventListener('DOMContentLoaded', () => {
  const params = new URLSearchParams(window.location.search);
  const txParam = params.get('tx');
  const blockParam = params.get('block');
  const addrParam = params.get('address');
  const isMempool = params.has('mempool') || params.get('view') === 'mempool' || params.get('tab') === 'mempool';

  if (isMempool) {
    inspectMempool();
  } else if (txParam) {
    inspectTx(txParam);
  } else if (blockParam) {
    inspectBlock(blockParam);
  } else if (addrParam) {
    inspectAddress(addrParam);
  } else {
    showLiveFeedView();
  }

  // Quick Chips
  const chipLatest = document.getElementById('chip-latest-block');
  if (chipLatest) {
    chipLatest.addEventListener('click', () => {
      if (currentTipHeight > 0) inspectBlock(currentTipHeight);
    });
  }

  const chipMempool = document.getElementById('chip-mempool');
  if (chipMempool) {
    chipMempool.addEventListener('click', () => {
      inspectMempool();
    });
  }

  const pillMempool = document.getElementById('pill-mempool');
  if (pillMempool) {
    pillMempool.style.cursor = 'pointer';
    pillMempool.addEventListener('click', () => {
      inspectMempool();
    });
  }

  const cardMempool = document.getElementById('card-mempool');
  if (cardMempool) {
    cardMempool.style.cursor = 'pointer';
    cardMempool.addEventListener('click', () => {
      inspectMempool();
    });
  }

  // Start telemetry loop
  pollExplorerTelemetry();
  setInterval(pollExplorerTelemetry, 3000);

  // Initialize Scroll To Top listener
  initScrollToTop();
});

// --- Scroll To Top Feature ---
function scrollToTop() {
  window.scrollTo({ top: 0, behavior: 'smooth' });
  if (document.documentElement) document.documentElement.scrollTo({ top: 0, behavior: 'smooth' });
  if (document.body) document.body.scrollTo({ top: 0, behavior: 'smooth' });
}

function initScrollToTop() {
  const btn = document.getElementById('scroll-to-top-btn');
  if (!btn) return;

  const checkScroll = () => {
    const scrollPos = window.pageYOffset || document.documentElement.scrollTop || document.body.scrollTop || 0;
    if (scrollPos > 220) {
      btn.classList.add('visible');
    } else {
      btn.classList.remove('visible');
    }
  };

  window.addEventListener('scroll', checkScroll, { passive: true });
  document.addEventListener('scroll', checkScroll, { passive: true });
  checkScroll();
}

// Exports for global inline onclicks
window.showLiveFeedView = showLiveFeedView;
window.inspectTx = inspectTx;
window.inspectBlock = inspectBlock;
window.inspectAddress = inspectAddress;
window.inspectMempool = inspectMempool;
window.handleExplorerSearch = handleExplorerSearch;
window.copyToClipboard = copyToClipboard;
window.scrollToTop = scrollToTop;
