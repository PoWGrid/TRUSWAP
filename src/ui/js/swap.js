// --- Exchange Spot Trading & Order Initiation ---
var activePercent = null;

function getMaxSpendableTru() {
  const confAtoms = Number((typeof vaultData !== 'undefined' && vaultData && (vaultData.confirmed_atoms || vaultData.balance_atoms)) || 0);
  const maxAtoms = Math.max(0, confAtoms - 1000); // Balance minus 1000 atoms buffer (0.00001 TRU)
  return Math.floor((maxAtoms / 1e8) * 10000) / 10000;
}

function getMaxSpendableUsdc() {
  const usdcBal = (typeof vaultData !== 'undefined' && vaultData && vaultData.base_usdc_balance !== undefined) 
    ? vaultData.base_usdc_balance 
    : (parseFloat(document.getElementById('base-usdc-balance')?.innerText?.replace(/,/g, '')) || 0);
  return Math.max(0, Math.floor(usdcBal * 100) / 100); // Exact amount on balance
}

function setSwapDirection(dir) {
  swapDirection = dir;
  activePercent = null;
  document.querySelectorAll('.trade-pct-btn').forEach(b => b.classList.remove('active'));
  const hintEl = document.getElementById('trade-pct-hint');
  if (hintEl) hintEl.style.display = 'none';

  const btnSell = document.getElementById('dir-sell-tru');
  const btnBuy = document.getElementById('dir-buy-tru');

  if (dir === 'sell') {
    if (btnSell) btnSell.className = "btn-action active btn-tab-sell";
    if (btnBuy) btnBuy.className = "btn-action btn-tab-buy";
  } else {
    if (btnBuy) btnBuy.className = "btn-action active btn-tab-buy";
    if (btnSell) btnSell.className = "btn-action btn-tab-sell";
  }

  // Toggle slider placement: when Buy -> under USDC (Total); when Sell -> under TRU (Amount)
  const sliderTruWrap = document.getElementById('slider-tru-container');
  const sliderUsdcWrap = document.getElementById('slider-usdc-container');
  if (dir === 'sell') {
    if (sliderTruWrap) sliderTruWrap.classList.remove('hidden-slider');
    if (sliderUsdcWrap) sliderUsdcWrap.classList.add('hidden-slider');
  } else {
    if (sliderUsdcWrap) sliderUsdcWrap.classList.remove('hidden-slider');
    if (sliderTruWrap) sliderTruWrap.classList.add('hidden-slider');
  }

  // Auto-clamp any existing input values to the limits of the new direction
  const price = parseFloat(document.getElementById('trade-price')?.value) || 0;
  const amountEl = document.getElementById('trade-amount-tru');
  const totalEl = document.getElementById('trade-total-usdc');
  if (amountEl && totalEl) {
    let amount = parseFloat(amountEl.value) || 0;
    let total = parseFloat(totalEl.value) || 0;

    if (dir === 'sell') {
      const maxTru = getMaxSpendableTru();
      if (amount > maxTru) {
        amount = maxTru;
        amountEl.value = maxTru > 0 ? maxTru.toFixed(4) : '';
        if (price > 0) totalEl.value = (price * amount).toFixed(2);
      }
    } else {
      const maxUsdc = getMaxSpendableUsdc();
      if (total > maxUsdc) {
        total = maxUsdc;
        totalEl.value = maxUsdc > 0 ? maxUsdc.toFixed(2) : '';
        if (price > 0) amountEl.value = (total / price).toFixed(4);
      }
    }
  }

  syncActiveSlider();
  updateTradeButtonText();
}

function onTradePriceInput() {
  const price = parseFloat(document.getElementById('trade-price').value) || 0;
  const hintEl = document.getElementById('trade-pct-hint');
  const amountEl = document.getElementById('trade-amount-tru');
  const totalEl = document.getElementById('trade-total-usdc');

  if (activePercent && swapDirection === 'buy') {
    let total = parseFloat(totalEl.value) || 0;
    const maxUsdc = getMaxSpendableUsdc();
    if (total > maxUsdc) {
      total = maxUsdc;
      totalEl.value = maxUsdc > 0 ? maxUsdc.toFixed(2) : '';
    }
    if (price > 0 && total > 0) {
      const truAmt = total / price;
      amountEl.value = truAmt.toFixed(4);
      if (hintEl && hintEl.style.display !== 'none') {
        hintEl.innerHTML = `${activePercent}% of ${maxUsdc.toFixed(2)} USDC = <strong>${total.toFixed(2)} USDC</strong> (${truAmt.toFixed(4)} TRU)`;
      }
    }
  } else {
    let amount = parseFloat(amountEl.value) || 0;
    if (swapDirection === 'sell') {
      const maxTru = getMaxSpendableTru();
      if (amount > maxTru) {
        amount = maxTru;
        amountEl.value = maxTru > 0 ? maxTru.toFixed(4) : '';
      }
    } else {
      const maxUsdc = getMaxSpendableUsdc();
      if (price > 0 && (amount * price) > maxUsdc) {
        amount = Math.floor((maxUsdc / price) * 10000) / 10000;
        amountEl.value = amount > 0 ? amount.toFixed(4) : '';
      }
    }

    if (price > 0 && amount > 0) {
      totalEl.value = (price * amount).toFixed(2);
    }
  }
  syncActiveSlider();
  updateTradeButtonText();
}

function onTradeAmountInput() {
  activePercent = null;
  const hintEl = document.getElementById('trade-pct-hint');
  const price = parseFloat(document.getElementById('trade-price').value) || 0;
  let amount = parseFloat(document.getElementById('trade-amount-tru').value) || 0;
  const totalEl = document.getElementById('trade-total-usdc');

  if (swapDirection === 'sell') {
    // Sell TRU: cap to spendable balance minus buffer
    const maxTru = getMaxSpendableTru();
    if (amount > maxTru) {
      amount = maxTru;
      document.getElementById('trade-amount-tru').value = maxTru > 0 ? maxTru.toFixed(4) : '';
      if (hintEl) {
        hintEl.style.display = 'block';
        hintEl.style.color = 'var(--tru-gold)';
        hintEl.innerHTML = `↩️ Capped to max spendable: <strong>${maxTru.toFixed(4)} TRU</strong> (Balance − 1000 atoms buffer)`;
      }
    } else if (hintEl) {
      hintEl.style.display = 'none';
    }
  } else {
    // Buy TRU: total USDC cannot exceed available USDC balance
    const maxUsdc = getMaxSpendableUsdc();
    if (price > 0 && (amount * price) > maxUsdc) {
      const maxTru = Math.floor((maxUsdc / price) * 10000) / 10000;
      amount = maxTru;
      document.getElementById('trade-amount-tru').value = maxTru > 0 ? maxTru.toFixed(4) : '';
      if (hintEl) {
        hintEl.style.display = 'block';
        hintEl.style.color = '#5d93ff';
        hintEl.innerHTML = `↩️ Capped to available budget: <strong>${maxUsdc.toFixed(2)} USDC</strong> (${maxTru.toFixed(4)} TRU)`;
      }
    } else if (hintEl) {
      hintEl.style.display = 'none';
    }
  }

  if (price > 0 && amount > 0) {
    totalEl.value = (price * amount).toFixed(2);
  } else if (amount === 0) {
    totalEl.value = '';
  }
  syncActiveSlider();
  updateTradeButtonText();
}

function onTradeTotalInput() {
  activePercent = null;
  const hintEl = document.getElementById('trade-pct-hint');

  const price = parseFloat(document.getElementById('trade-price').value) || 0;
  let total = parseFloat(document.getElementById('trade-total-usdc').value) || 0;
  const amountEl = document.getElementById('trade-amount-tru');

  if (swapDirection === 'buy') {
    // Buy TRU: total USDC cannot exceed available USDC balance
    const maxUsdc = getMaxSpendableUsdc();
    if (total > maxUsdc) {
      total = maxUsdc;
      document.getElementById('trade-total-usdc').value = maxUsdc > 0 ? maxUsdc.toFixed(2) : '';
      if (hintEl) {
        hintEl.style.display = 'block';
        hintEl.style.color = '#5d93ff';
        hintEl.innerHTML = `↩️ Capped to available USDC balance: <strong>${maxUsdc.toFixed(2)} USDC</strong>`;
      }
    } else if (hintEl) {
      hintEl.style.display = 'none';
    }
  } else {
    // Sell TRU: cap total to available TRU equivalent
    const maxTru = getMaxSpendableTru();
    const maxTotalUsdc = price > 0 ? (maxTru * price) : 0;
    if (price > 0 && total > maxTotalUsdc) {
      total = maxTotalUsdc;
      document.getElementById('trade-total-usdc').value = maxTotalUsdc > 0 ? maxTotalUsdc.toFixed(2) : '';
      if (hintEl) {
        hintEl.style.display = 'block';
        hintEl.style.color = 'var(--tru-gold)';
        hintEl.innerHTML = `↩️ Capped to max spendable: <strong>${maxTru.toFixed(4)} TRU</strong> ($${maxTotalUsdc.toFixed(2)} USDC)`;
      }
    } else if (hintEl) {
      hintEl.style.display = 'none';
    }
  }

  if (price > 0 && total > 0) {
    amountEl.value = (total / price).toFixed(4);
  } else if (total === 0) {
    amountEl.value = '';
  }
  syncActiveSlider();
  updateTradeButtonText();
}

function updateSliderVisuals(sliderId, pct) {
  const el = document.getElementById(sliderId);
  if (el) {
    el.value = pct;
    el.style.setProperty('--slider-pct', `${pct}%`);
    const container = el.closest('.trade-slider-container');
    if (container) {
      container.querySelectorAll('.slider-mark').forEach(m => {
        const markVal = parseInt(m.innerText);
        if (markVal === pct) {
          m.classList.add('active');
        } else {
          m.classList.remove('active');
        }
      });
    }
  }
}

function onSliderUsdcInput(val) {
  const pct = parseInt(val) || 0;
  updateSliderVisuals('trade-slider-usdc', pct);
  setTradePercent(pct);
}

function onSliderTruInput(val) {
  const pct = parseInt(val) || 0;
  updateSliderVisuals('trade-slider-tru', pct);
  setTradePercent(pct);
}

function snapSliderUsdc(pct) {
  updateSliderVisuals('trade-slider-usdc', pct);
  setTradePercent(pct);
}

function snapSliderTru(pct) {
  updateSliderVisuals('trade-slider-tru', pct);
  setTradePercent(pct);
}

function syncActiveSlider() {
  if (swapDirection === 'sell') {
    const spendable = getMaxSpendableTru();
    const amt = parseFloat(document.getElementById('trade-amount-tru')?.value) || 0;
    const pct = spendable > 0 ? Math.min(100, Math.max(0, Math.round((amt / spendable) * 100))) : 0;
    updateSliderVisuals('trade-slider-tru', pct);
  } else {
    const usdcBal = getMaxSpendableUsdc();
    const total = parseFloat(document.getElementById('trade-total-usdc')?.value) || 0;
    const pct = usdcBal > 0 ? Math.min(100, Math.max(0, Math.round((total / usdcBal) * 100))) : 0;
    updateSliderVisuals('trade-slider-usdc', pct);
  }
}

function setTradePercent(pct) {
  activePercent = pct;
  const price = parseFloat(document.getElementById('trade-price').value) || 1.0;
  const hintEl = document.getElementById('trade-pct-hint');

  if (swapDirection === 'sell') {
    updateSliderVisuals('trade-slider-tru', pct);
    const spendable = getMaxSpendableTru();
    const amt = spendable * (pct / 100);
    const safeAmt = (Math.floor(amt * 10000) / 10000).toFixed(4);
    document.getElementById('trade-amount-tru').value = safeAmt;
    const totalUsdc = (price > 0) ? (Number(safeAmt) * price) : 0;
    if (price > 0) {
      document.getElementById('trade-total-usdc').value = totalUsdc.toFixed(2);
    }
    if (hintEl) {
      hintEl.style.display = 'block';
      hintEl.style.color = '#fbbf24';
      hintEl.innerHTML = `${pct}% of ${spendable.toFixed(4)} TRU = <strong>${safeAmt} TRU</strong> ($${totalUsdc.toFixed(2)} USDC)`;
    }
  } else {
    updateSliderVisuals('trade-slider-usdc', pct);
    const usdcBal = getMaxSpendableUsdc();
    const totalUsdc = usdcBal * (pct / 100);
    document.getElementById('trade-total-usdc').value = totalUsdc.toFixed(2);
    const truAmt = price > 0 ? (totalUsdc / price) : 0;
    if (price > 0) {
      document.getElementById('trade-amount-tru').value = truAmt.toFixed(4);
    }
    if (hintEl) {
      hintEl.style.display = 'block';
      hintEl.style.color = '#38bdf8';
      hintEl.innerHTML = `${pct}% of ${usdcBal.toFixed(2)} USDC = <strong>${totalUsdc.toFixed(2)} USDC</strong> (${truAmt.toFixed(4)} TRU)`;
    }
  }
  updateTradeButtonText();
}

function fillBestPrice() {
  const seenOrderKeys = new Set();
  const allOrders = [];
  for (const o of ordersMap.values()) {
    if (!o || !o.isLive) continue;
    const cleanHash = (o.hash160 || '').replace(/^0x/, '').toLowerCase();
    const uid = cleanHash || o.id;
    if (!uid || seenOrderKeys.has(uid)) continue;
    seenOrderKeys.add(uid);
    allOrders.push(o);
  }
  if (swapDirection === 'buy') {
    // When buying, best price is lowest ask
    const asks = allOrders.filter(o => o.side === 'SELL_TRU');
    if (asks.length > 0) {
      asks.sort((a, b) => ((a.usdcAmount / a.truAmount) - (b.usdcAmount / b.truAmount)));
      const p = (asks[0].usdcAmount / asks[0].truAmount);
      if (p > 0) document.getElementById('trade-price').value = p.toFixed(4);
    }
  } else {
    // When selling, best price is highest bid
    const bids = allOrders.filter(o => o.side === 'BUY_TRU');
    if (bids.length > 0) {
      bids.sort((a, b) => ((b.usdcAmount / b.truAmount) - (a.usdcAmount / a.truAmount)));
      const p = (bids[0].usdcAmount / bids[0].truAmount);
      if (p > 0) document.getElementById('trade-price').value = p.toFixed(4);
    }
  }
  onTradePriceInput();
}

function fillTradeForm(price, amount, side) {
  if (price && price > 0) {
    const priceEl = document.getElementById('trade-price');
    if (priceEl) priceEl.value = Number(price).toFixed(4);
  }
  if (amount && amount > 0) {
    const amtEl = document.getElementById('trade-amount-tru');
    if (amtEl) amtEl.value = Number(amount).toFixed(4);
  }
  const p = parseFloat(document.getElementById('trade-price')?.value) || 0;
  const a = parseFloat(document.getElementById('trade-amount-tru')?.value) || 0;
  const totalEl = document.getElementById('trade-total-usdc');
  if (totalEl && p > 0 && a > 0) {
    totalEl.value = (p * a).toFixed(2);
  }
  if (side) {
    setSwapDirection(side);
  } else {
    updateTradeButtonText();
  }
}

function updateTradeButtonText() {
  const mainBtn = document.getElementById('btn-initiate-swap');
  if (!mainBtn) return;
  const amount = parseFloat(document.getElementById('trade-amount-tru')?.value) || 0;
  const total = parseFloat(document.getElementById('trade-total-usdc')?.value) || 0;

  const availUsdc = (typeof vaultData !== 'undefined' && vaultData && vaultData.base_usdc_balance !== undefined) ? vaultData.base_usdc_balance : 0;
  const availEth = (typeof vaultData !== 'undefined' && vaultData && vaultData.base_eth_balance !== undefined) ? vaultData.base_eth_balance : 0;
  const confAtoms = Number((typeof vaultData !== 'undefined' && vaultData && (vaultData.confirmed_atoms || vaultData.balance_atoms)) || 0);
  const availTru = confAtoms / 1e8;
  const requiredTruFee = 0.000005; // 500 atoms: 250 lock tx fee + 250 claim fee surplus

  if (swapDirection === 'buy') {
    mainBtn.className = "btn-action btn-swap-buy";
    const minEthGasBuffer = 0.00003; // Dynamic Base L2 execution fee + L1 blob data fee with buffer
    if (amount <= 0 || total <= 0) {
      mainBtn.disabled = true;
      mainBtn.innerText = "Enter Amount & Price";
      mainBtn.title = "Please enter both TRU amount and price";
    } else if (total > availUsdc) {
      mainBtn.disabled = true;
      mainBtn.innerText = `Insufficient USDC (${availUsdc.toFixed(2)} avail)`;
      mainBtn.title = `Your Base Vault has ${availUsdc.toFixed(2)} USDC, but this order requires ${total.toFixed(2)} USDC`;
    } else if (availEth < minEthGasBuffer) {
      mainBtn.disabled = true;
      mainBtn.innerText = `Insufficient ETH for Gas (<${minEthGasBuffer.toFixed(5)})`;
      mainBtn.title = `Estimated ~${minEthGasBuffer.toFixed(5)} ETH required for Base L2 gas fee (vault has ${availEth.toFixed(5)} ETH)`;
    } else {
      mainBtn.disabled = false;
      mainBtn.innerText = `Buy ${amount.toFixed(4)} TRU (${total.toFixed(2)} USDC)`;
      mainBtn.title = "Place Buy order in orderbook";
    }
  } else {
    mainBtn.className = "btn-action btn-swap-sell";
    const minEthClaimGasBuffer = 0.00002; // Dynamic Base L2 fee to claim USDC with buffer
    if (amount <= 0 || total <= 0) {
      mainBtn.disabled = true;
      mainBtn.innerText = "Enter Amount & Price";
      mainBtn.title = "Please enter both TRU amount and price";
    } else if (amount + requiredTruFee > availTru) {
      mainBtn.disabled = true;
      mainBtn.innerText = `Insufficient TRU (${availTru.toFixed(4)} avail)`;
      mainBtn.title = `Spendable TRU: ${availTru.toFixed(4)} TRU, but order requires ${amount.toFixed(4)} TRU + ~0.000005 fee buffer (500 atoms)`;
    } else if (availEth < minEthClaimGasBuffer) {
      mainBtn.disabled = true;
      mainBtn.innerText = `Insufficient ETH for Gas (<${minEthClaimGasBuffer.toFixed(5)})`;
      mainBtn.title = `Estimated ~${minEthClaimGasBuffer.toFixed(5)} ETH required to claim USDC on Base (vault has ${availEth.toFixed(5)} ETH)`;
    } else {
      mainBtn.disabled = false;
      mainBtn.innerText = `Sell ${amount.toFixed(4)} TRU (${total.toFixed(2)} USDC)`;
      mainBtn.title = "Place Sell order in orderbook";
    }
  }
  updateAvailableBalanceLabel();
}

function updateAvailableBalanceLabel() {
  const el = document.getElementById('trade-available-balance');
  if (!el) return;
  if (swapDirection === 'buy') {
    const usdcBal = (vaultData && vaultData.base_usdc_balance !== undefined) ? vaultData.base_usdc_balance.toFixed(2) : (document.getElementById('base-usdc-balance')?.innerText || "0.00");
    el.innerHTML = `<span>Available:</span><strong style="color: #38bdf8; margin: 0 4px;">${usdcBal} USDC</strong><span style="color: #fbbf24;">(Base Vault)</span>`;
  } else {
    const confAtoms = Number(vaultData.confirmed_atoms || vaultData.balance_atoms || 0);
    const truBal = (confAtoms / 1e8).toFixed(4);
    el.innerHTML = `<span>Available:</span><strong style="color: var(--tru-gold); margin: 0 4px;">${truBal} TRU</strong><span style="color: #fbbf24;">(TRU Vault)</span>`;
  }
}

var currentTakeOrder = null;

async function initiateHtlcOffer() {
  const price = parseFloat(document.getElementById('trade-price').value) || 0;
  const truAmt = parseFloat(document.getElementById('trade-amount-tru').value) || 0;
  const usdcAmt = parseFloat(document.getElementById('trade-total-usdc').value) || (price * truAmt);

  if (price <= 0 || truAmt <= 0 || usdcAmt <= 0) {
    showToast("⚠️ Please specify both Price and Amount (TRU) for your order.", "warning");
    return;
  }
  const tlEl = document.getElementById('swap-timelock');
  const tlVal = (tlEl && tlEl.value) ? parseInt(tlEl.value) : 240;

  if (swapDirection === 'sell') {
    const confAtoms = BigInt(vaultData.confirmed_atoms !== undefined ? vaultData.confirmed_atoms : (vaultData.balance_atoms || 0));
    const neededAtoms = BigInt(Math.floor(truAmt * 1e8));
    if (neededAtoms > confAtoms) {
      showToast(`⚠️ Insufficient confirmed TRU balance. Spendable: ${(Number(confAtoms)/1e8).toFixed(8)} TRU.`, "warning");
      return;
    }
  } else {
    const availUsdc = (vaultData && vaultData.base_usdc_balance !== undefined) ? vaultData.base_usdc_balance : 0;
    if (usdcAmt > availUsdc) {
      showToast(`⚠️ Insufficient Base Vault USDC balance. Have: ${availUsdc.toFixed(2)} USDC, Requested: ${usdcAmt.toFixed(2)} USDC.`, "warning");
      return;
    }
  }

  const btn = document.getElementById('btn-initiate-swap');
  if (btn) {
    btn.disabled = true;
    btn.innerText = "Placing Order...";
  }

  try {
    const data = await sendSocketAction('create_order', {
      side: swapDirection === 'sell' ? 'SELL_TRU' : 'BUY_TRU',
      amount_tru: truAmt,
      amount_usdc: usdcAmt,
      timelock_hours: 4,
      maker_base_address: (vaultData && vaultData.base_address) ? vaultData.base_address : null,
    });

    if (data.success) {
      showToast(`✓ Order placed in book! Hash160: 0x${data.hash160.slice(0, 8)}...`);
      if (typeof addMyCreatedOrder === 'function') {
        addMyCreatedOrder(data.hash160, data.swap_id);
      }
      if (typeof renderOrderbook === 'function') {
        renderOrderbook();
      }

      document.getElementById('trade-amount-tru').value = '';
      document.getElementById('trade-total-usdc').value = '';
      updateTradeButtonText();
    } else {
      showToast("⚠️ Failed to place order: " + (data.error || "Unknown error"), "error");
    }
  } catch (err) {
    showToast("⚠️ Error placing order: " + err.message, "error");
  } finally {
    if (btn) {
      btn.disabled = false;
      updateTradeButtonText();
    }
  }
}

async function takeOrder(hash160Hex) {
  let order = ordersMap.get(hash160Hex);
  if (!order && typeof fetchOrderbookFromApi === 'function') {
    await fetchOrderbookFromApi();
    order = ordersMap.get(hash160Hex);
  }

  if (!order) {
    showToast("⚠️ Order not found or has expired.", "warning");
    return;
  }

  // Ensure positive USDC amount
  if (!order.usdcAmount || order.usdcAmount <= 0) {
    order.usdcAmount = Math.max(0.01, (order.price > 0 ? order.price : 1.0) * order.truAmount);
  }

  // Populate and Open Modal FIRST so user always sees the trade
  currentTakeOrder = order;
  const modal = document.getElementById('modal-take-order');
  if (!modal) {
    executeTakeSellOrderDirect(order);
    return;
  }

  const isMakerSelling = (order.side === 'SELL_TRU');
  const modalTitle = document.getElementById('take-modal-title');
  const payLabel = document.getElementById('take-modal-pay-label');
  const payAmount = document.getElementById('take-modal-pay-amount');
  const recvLabel = document.getElementById('take-modal-receive-label');
  const recvAmount = document.getElementById('take-modal-receive-amount');
  const btnTakeText = document.getElementById('btn-take-swap-text');
  const btnConfirm = document.getElementById('btn-confirm-take-swap');
  const statusBox = document.getElementById('take-modal-status-box');
  const statusText = document.getElementById('take-modal-status-text');

  if (isMakerSelling) {
    if (modalTitle) modalTitle.innerText = "Buy TRU";
    if (payLabel) payLabel.innerText = "YOU SEND (USDC ON BASE)";
    if (payAmount) payAmount.innerText = `${order.usdcAmount.toFixed(2)} USDC`;
    if (recvLabel) recvLabel.innerText = "YOU RECEIVE (TRU)";
    if (recvAmount) recvAmount.innerText = `${order.truAmount.toFixed(4)} TRU`;
    if (btnTakeText) btnTakeText.innerText = `Buy ${order.truAmount.toFixed(4)} TRU`;
  } else {
    if (modalTitle) modalTitle.innerText = "Sell TRU";
    if (payLabel) payLabel.innerText = "YOU SEND (TRU)";
    if (payAmount) payAmount.innerText = `${order.truAmount.toFixed(4)} TRU`;
    if (recvLabel) recvLabel.innerText = "YOU RECEIVE (USDC ON BASE)";
    if (recvAmount) recvAmount.innerText = `${order.usdcAmount.toFixed(2)} USDC`;
    if (btnTakeText) btnTakeText.innerText = `Sell ${order.truAmount.toFixed(4)} TRU`;
  }
  
  const beneficiary = order.makerBaseAddress || "—";
  const elBen = document.getElementById('take-modal-beneficiary');
  if (elBen) {
    elBen.innerText = beneficiary;
    elBen.title = beneficiary;
  }
  
  const vAddr = (typeof vaultData !== 'undefined' && vaultData && vaultData.address) ? vaultData.address : "Local Vault";
  const elVAddr = document.getElementById('take-modal-vault-addr');
  if (elVAddr) {
    elVAddr.innerText = vAddr;
    elVAddr.title = vAddr;
  }

  const hLock = order.hash160 || order.hash160_hex || "0x0";
  const elHLock = document.getElementById('take-modal-hashlock');
  if (elHLock) {
    elHLock.innerText = `0x${hLock.slice(0, 10)}...${hLock.slice(-8)}`;
    elHLock.title = `0x${hLock}`;
  }

  const rate = (order.truAmount > 0) ? (order.usdcAmount / order.truAmount).toFixed(4) : "1.0000";
  const elRate = document.getElementById('take-modal-rate');
  if (elRate) {
    elRate.innerText = `1 TRU = ${rate} USDC`;
  }

  const availUsdc = (typeof vaultData !== 'undefined' && vaultData && vaultData.base_usdc_balance !== undefined) ? vaultData.base_usdc_balance : 0;
  const availEth = (typeof vaultData !== 'undefined' && vaultData && vaultData.base_eth_balance !== undefined) ? vaultData.base_eth_balance : 0;
  const confAtoms = Number((typeof vaultData !== 'undefined' && vaultData && (vaultData.confirmed_atoms || vaultData.balance_atoms)) || 0);
  const availTru = confAtoms / 1e8;
  const requiredTruFee = 0.000005; // 500 atoms: 250 lock tx fee + 250 claim fee surplus

  let canTake = true;
  let takeDisabledReason = '';

  if (isMakerSelling) {
    // Taker buys TRU by paying USDC on Base
    const minEthGasBuffer = 0.00003;
    if (order.usdcAmount > availUsdc) {
      canTake = false;
      takeDisabledReason = `Insufficient USDC (${availUsdc.toFixed(2)} available, need ${order.usdcAmount.toFixed(2)})`;
    } else if (availEth < minEthGasBuffer) {
      canTake = false;
      takeDisabledReason = `Insufficient Base ETH for Gas (<${minEthGasBuffer.toFixed(5)} ETH, have ${availEth.toFixed(5)})`;
    }
  } else {
    // Taker sells TRU by locking TRU and claiming USDC on Base
    const minEthClaimGasBuffer = 0.00002;
    if (order.truAmount + requiredTruFee > availTru) {
      canTake = false;
      takeDisabledReason = `Insufficient TRU (${availTru.toFixed(4)} available, need ${(order.truAmount + requiredTruFee).toFixed(4)})`;
    } else if (availEth < minEthClaimGasBuffer) {
      canTake = false;
      takeDisabledReason = `Insufficient Base ETH for Gas (<${minEthClaimGasBuffer.toFixed(5)} ETH, have ${availEth.toFixed(5)})`;
    }
  }

  if (btnConfirm) {
    btnConfirm.disabled = !canTake;
  }
  if (!canTake && btnTakeText) {
    btnTakeText.innerText = takeDisabledReason;
  } else if (btnTakeText) {
    btnTakeText.innerText = isMakerSelling ? `Buy ${order.truAmount.toFixed(4)} TRU` : `Sell ${order.truAmount.toFixed(4)} TRU`;
  }

  if (!canTake && statusBox && statusText) {
    statusBox.style.display = 'block';
    statusBox.style.background = 'rgba(239, 68, 68, 0.12)';
    statusBox.style.color = '#f87171';
    statusBox.style.border = '1px solid rgba(239, 68, 68, 0.3)';
    statusText.innerText = `⚠️ ${takeDisabledReason}. Please deposit funds into your vault first.`;
  } else if (statusBox) {
    statusBox.style.display = 'none';
  }

  document.body.classList.add('modal-open');
  modal.classList.add('open');
  modal.style.display = 'flex';
}

function closeTakeOrderModal() {
  const modal = document.getElementById('modal-take-order');
  if (modal) {
    modal.classList.remove('open');
    modal.style.display = 'none';
  }
  document.body.classList.remove('modal-open');
  currentTakeOrder = null;
}

async function confirmExecuteTakeSwap() {
  if (!currentTakeOrder) return;
  const order = currentTakeOrder;

  const btnConfirm = document.getElementById('btn-confirm-take-swap');
  const btnText = document.getElementById('btn-take-swap-text');
  const statusBox = document.getElementById('take-modal-status-box');
  const statusText = document.getElementById('take-modal-status-text');

  const setModalStatus = (msg) => {
    if (statusBox && statusText) {
      statusBox.style.display = 'block';
      statusText.innerText = msg;
    }
    if (typeof showToast === 'function') showToast(msg);
  };

  const isMakerSelling = (order.side === 'SELL_TRU' || order.side === 'SellTruForBaseUsdc' || order.side === 2);
  const defaultBtnText = isMakerSelling ? `Buy ${order.truAmount.toFixed(4)} TRU` : `Sell ${order.truAmount.toFixed(4)} TRU`;

  try {
    if (btnConfirm) btnConfirm.disabled = true;
    if (btnText) btnText.innerText = 'Initiating Autonomous Swap...';
    setModalStatus('Submitting order take request to daemon...');

    const payload = {
      hash160: order.hash160 || order.hash160_hex || '',
      order_id: order.id || order.order_id || ''
    };

    const data = await sendSocketAction('take_order', payload);
    if (!data.success) {
      throw new Error(data.error || 'Failed to take order');
    }

    setModalStatus('🎉 Swap initiated! Moved to In-Progress tracker: awaiting TRU network confirmations (0/6)...');
    if (typeof showToast === 'function') {
      showToast(data.message || '🎉 Swap taken successfully!');
    }

    if (typeof registerPendingTakeSwap === 'function') {
      registerPendingTakeSwap(order);
    }

    setTimeout(() => {
      closeTakeOrderModal();
    }, 1500);

  } catch (err) {
    console.error('Take swap error:', err);
    showToast('⚠️ Take Swap failed: ' + (err.reason || err.message || err), 'error');
    if (btnConfirm) btnConfirm.disabled = false;
    if (btnText) btnText.innerText = defaultBtnText;
  }
}

async function claimBaseHtlc(swapId, hash160) {
  try {
    if (typeof showToast === 'function') showToast('Requesting Base claim from daemon...');
    const data = await sendSocketAction('claim_base_htlc', { swap_id: swapId || '', hash160: hash160 || '' });
    if (data.success) {
      if (typeof showToast === 'function') showToast(`🎉 USDC Claimed on Base! TX: ${data.tx_hash ? data.tx_hash.slice(0, 10) : ''}...`);
      if (typeof renderTradeHistory === 'function') renderTradeHistory();
    } else {
      showToast('⚠️ Claim failed: ' + (data.error || 'Unknown error'), 'error');
    }
  } catch (err) {
    console.error('Claim error:', err);
    showToast('⚠️ Claim failed: ' + (err.reason || err.message || err), 'error');
  }
}

let isLockingBase = false;

async function depositBaseHtlcForBuy(cleanHash, usdcAmt, takerBaseAddr, truTxid, truVout, truAmt) {
  if (isLockingBase) return;
  const cleanH = (cleanHash || '').replace(/^0x/, '').toLowerCase();

  if (typeof historicalLedgerData !== 'undefined' && historicalLedgerData && historicalLedgerData.swaps) {
    const existing = historicalLedgerData.swaps.find(s => (s.hash160_hex || '').toLowerCase() === cleanH);
    if (existing && (existing.status === 'Claimed' || existing.status === 'Refunded' || existing.is_spent || existing.base_claimed || existing.vault_status === 1 || existing.vault_status === 2)) {
      showToast('⚠️ Swap order has already been completed or settled. Cannot lock again.', 'warning');
      return;
    }
  }

  isLockingBase = true;
  try {
    if (typeof showToast === 'function') showToast('Requesting USDC lock on Base from daemon...');
    const data = await sendSocketAction('lock_base_htlc', {
      hash160: cleanH,
      amount_usdc: Number(usdcAmt),
      beneficiary: takerBaseAddr
    });
    if (data.success) {
      if (typeof showToast === 'function') showToast(`🎉 USDC Locked on Base! TX: ${data.tx_hash ? data.tx_hash.slice(0, 10) : ''}...`);
    } else {
      showToast('⚠️ Base lock failed: ' + (data.error || 'Unknown error'), 'error');
    }
  } catch (err) {
    console.error('depositBaseHtlcForBuy error:', err);
    showToast('⚠️ Deposit failed: ' + (err.reason || err.message || err), 'error');
  } finally {
    isLockingBase = false;
  }
}

async function claimTruHtlc(cleanHash, truTxid, truVout, truAmt) {
  if (typeof showToast === 'function') showToast("Claiming TRU to your vault...");
  try {
    const data = await sendSocketAction('claim_vault_htlc', {
      hash160: cleanHash,
      txid: truTxid,
      vout: truVout
    });
    if (data.success) {
      if (typeof showToast === 'function') showToast(`🎉 ${truAmt ? Number(truAmt).toFixed(4) : ''} TRU Claimed! TXID: ${data.txid.slice(0, 10)}...`);
    } else {
      showToast("⚠️ TRU Claim failed: " + (data.error || "Unknown error"), "error");
    }
  } catch (err) {
    console.error('claimTruHtlc error:', err);
    showToast("⚠️ TRU Claim failed: " + (err.reason || err.message || err), "error");
  }
}

// Global exports for inline HTML onclick handlers
window.takeOrder = takeOrder;
window.closeTakeOrderModal = closeTakeOrderModal;
window.confirmExecuteTakeSwap = confirmExecuteTakeSwap;
window.claimBaseHtlc = claimBaseHtlc;
window.depositBaseHtlcForBuy = depositBaseHtlcForBuy;
window.claimTruHtlc = claimTruHtlc;
window.fillTradeForm = fillTradeForm;
window.setSwapDirection = setSwapDirection;
window.onTradePriceInput = onTradePriceInput;
window.onTradeAmountInput = onTradeAmountInput;
window.onTradeTotalInput = onTradeTotalInput;
window.setTradePercent = setTradePercent;
window.fillBestPrice = fillBestPrice;
window.initiateHtlcOffer = initiateHtlcOffer;
window.onSliderUsdcInput = onSliderUsdcInput;
window.onSliderTruInput = onSliderTruInput;
window.snapSliderUsdc = snapSliderUsdc;
window.snapSliderTru = snapSliderTru;

