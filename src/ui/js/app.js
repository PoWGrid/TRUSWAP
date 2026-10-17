// --- Application Lifecycle & UI Event Handlers ---
window.addEventListener('DOMContentLoaded', () => {
  const urlParams = new URLSearchParams(window.location.search);
  const urlTok = urlParams.get('token');
  sessionToken = (urlTok && urlTok.length > 0) ? urlTok : (window.SERVER_SESSION_TOKEN || sessionToken || "");
  initWebSocket();
  // Screen Wake Lock API to prevent screen timeout / sleep while active
  let wakeLock = null;
  const acquireWakeLock = async () => {
    try {
      if ('wakeLock' in navigator) {
        wakeLock = await navigator.wakeLock.request('screen');
        console.log('[TRUSWAP-POWER] Screen Wake Lock acquired');
        wakeLock.addEventListener('release', () => {
          console.log('[TRUSWAP-POWER] Screen Wake Lock released');
        });
      }
    } catch (e) {
      console.warn('[TRUSWAP-POWER] Screen Wake Lock failed:', e);
    }
  };
  acquireWakeLock();
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') {
      acquireWakeLock();
    }
  });

  // Strict backdrop close for Vault Modal: only close if click starts AND ends on backdrop
  const vaultModalEl = document.getElementById('vault-modal');
  if (vaultModalEl) {
    let isBackdropMouseDown = false;
    let isBackdropTouchStart = false;

    vaultModalEl.addEventListener('wheel', (e) => {
      if (e.target === vaultModalEl) {
        e.preventDefault();
      }
    }, { passive: false });

    vaultModalEl.addEventListener('mousedown', (e) => {
      isBackdropMouseDown = (e.button === 0 && e.target === vaultModalEl);
    });

    vaultModalEl.addEventListener('mouseup', (e) => {
      if (isBackdropMouseDown && e.button === 0 && e.target === vaultModalEl) {
        closeVaultModal();
      }
      isBackdropMouseDown = false;
    });

    vaultModalEl.addEventListener('touchstart', (e) => {
      isBackdropTouchStart = (e.target === vaultModalEl);
    }, { passive: true });

    vaultModalEl.addEventListener('touchend', (e) => {
      if (isBackdropTouchStart && e.target === vaultModalEl) {
        closeVaultModal();
      }
      isBackdropTouchStart = false;
    });

    window.addEventListener('mouseup', () => {
      isBackdropMouseDown = false;
    });
    window.addEventListener('touchend', () => {
      isBackdropTouchStart = false;
    });
  }

  // Strict backdrop close for Base Vault Modal
  const baseModalEl = document.getElementById('base-vault-modal');
  if (baseModalEl) {
    let isBaseMouseDown = false;
    let isBaseTouchStart = false;

    baseModalEl.addEventListener('wheel', (e) => {
      if (e.target === baseModalEl) {
        e.preventDefault();
      }
    }, { passive: false });

    baseModalEl.addEventListener('mousedown', (e) => {
      isBaseMouseDown = (e.button === 0 && e.target === baseModalEl);
    });

    baseModalEl.addEventListener('mouseup', (e) => {
      if (isBaseMouseDown && e.button === 0 && e.target === baseModalEl) {
        closeBaseModal();
      }
      isBaseMouseDown = false;
    });

    baseModalEl.addEventListener('touchstart', (e) => {
      isBaseTouchStart = (e.target === baseModalEl);
    }, { passive: true });

    baseModalEl.addEventListener('touchend', (e) => {
      if (isBaseTouchStart && e.target === baseModalEl) {
        closeBaseModal();
      }
      isBaseTouchStart = false;
    });

    window.addEventListener('mouseup', () => {
      isBaseMouseDown = false;
    });
    window.addEventListener('touchend', () => {
      isBaseTouchStart = false;
    });
  }

  // Strict backdrop close for Take Order Modal: only close if click starts AND ends on backdrop
  const takeModalEl = document.getElementById('modal-take-order');
  if (takeModalEl) {
    let isTakeMouseDown = false;
    let isTakeTouchStart = false;

    takeModalEl.addEventListener('wheel', (e) => {
      if (e.target === takeModalEl) {
        e.preventDefault();
      }
    }, { passive: false });

    takeModalEl.addEventListener('mousedown', (e) => {
      isTakeMouseDown = (e.button === 0 && e.target === takeModalEl);
    });

    takeModalEl.addEventListener('mouseup', (e) => {
      if (isTakeMouseDown && e.button === 0 && e.target === takeModalEl) {
        closeTakeOrderModal();
      }
      isTakeMouseDown = false;
    });

    takeModalEl.addEventListener('touchstart', (e) => {
      isTakeTouchStart = (e.target === takeModalEl);
    }, { passive: true });

    takeModalEl.addEventListener('touchend', (e) => {
      if (isTakeTouchStart && e.target === takeModalEl) {
        closeTakeOrderModal();
      }
      isTakeTouchStart = false;
    });

    window.addEventListener('mouseup', () => {
      isTakeMouseDown = false;
    });
    window.addEventListener('touchend', () => {
      isTakeTouchStart = false;
    });
  }

  // Strict backdrop close for TRU Tx Explorer Modal
  const truTxModalEl = document.getElementById('tru-tx-modal');
  if (truTxModalEl) {
    let isTxMouseDown = false;
    let isTxTouchStart = false;

    truTxModalEl.addEventListener('wheel', (e) => {
      if (e.target === truTxModalEl) {
        e.preventDefault();
      }
    }, { passive: false });

    truTxModalEl.addEventListener('mousedown', (e) => {
      isTxMouseDown = (e.button === 0 && e.target === truTxModalEl);
    });

    truTxModalEl.addEventListener('mouseup', (e) => {
      if (isTxMouseDown && e.button === 0 && e.target === truTxModalEl) {
        closeTruTxModal();
      }
      isTxMouseDown = false;
    });

    truTxModalEl.addEventListener('touchstart', (e) => {
      isTxTouchStart = (e.target === truTxModalEl);
    }, { passive: true });

    truTxModalEl.addEventListener('touchend', (e) => {
      if (isTxTouchStart && e.target === truTxModalEl) {
        closeTruTxModal();
      }
      isTxTouchStart = false;
    });

    window.addEventListener('mouseup', () => {
      isTxMouseDown = false;
    });
    window.addEventListener('touchend', () => {
      isTxTouchStart = false;
    });
  }

  // Strict backdrop close for Process Audit Modal: only close if click starts AND ends on backdrop
  const auditModalEl = document.getElementById('automation-audit-modal');
  if (auditModalEl) {
    let isAuditMouseDown = false;
    let isAuditTouchStart = false;

    auditModalEl.addEventListener('wheel', (e) => {
      if (e.target === auditModalEl) {
        e.preventDefault();
      }
    }, { passive: false });

    auditModalEl.addEventListener('mousedown', (e) => {
      isAuditMouseDown = (e.button === 0 && e.target === auditModalEl);
    });

    auditModalEl.addEventListener('mouseup', (e) => {
      if (isAuditMouseDown && e.button === 0 && e.target === auditModalEl) {
        if (typeof closeAutomationAuditModal === 'function') closeAutomationAuditModal();
      }
      isAuditMouseDown = false;
    });

    auditModalEl.addEventListener('touchstart', (e) => {
      isAuditTouchStart = (e.target === auditModalEl);
    }, { passive: true });

    auditModalEl.addEventListener('touchend', (e) => {
      if (isAuditTouchStart && e.target === auditModalEl) {
        if (typeof closeAutomationAuditModal === 'function') closeAutomationAuditModal();
      }
      isAuditTouchStart = false;
    });

    window.addEventListener('mouseup', () => {
      isAuditMouseDown = false;
    });
    window.addEventListener('touchend', () => {
      isAuditTouchStart = false;
    });

    // Close button ('X') explicit click and touchend handlers
    const closeBtn = document.getElementById('audit-modal-close-btn') || auditModalEl.querySelector('.audit-modal-close-btn');
    if (closeBtn) {
      closeBtn.addEventListener('click', (e) => {
        e.preventDefault();
        e.stopPropagation();
        if (typeof closeAutomationAuditModal === 'function') closeAutomationAuditModal();
      });
      closeBtn.addEventListener('touchend', (e) => {
        e.preventDefault();
        e.stopPropagation();
        if (typeof closeAutomationAuditModal === 'function') closeAutomationAuditModal();
      });
    }
  }

  // Escape key closes modals
  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      const modal = document.getElementById('vault-modal');
      if (modal && modal.classList.contains('open')) {
        closeVaultModal();
      }
      const baseModal = document.getElementById('base-vault-modal');
      if (baseModal && baseModal.classList.contains('open')) {
        closeBaseModal();
      }
      const takeModal = document.getElementById('modal-take-order');
      if (takeModal && takeModal.style.display !== 'none') {
        closeTakeOrderModal();
      }
      const truTxModal = document.getElementById('tru-tx-modal');
      if (truTxModal && (truTxModal.classList.contains('open') || truTxModal.style.display !== 'none')) {
        closeTruTxModal();
      }
      const auditModal = document.getElementById('automation-audit-modal');
      if (auditModal && (auditModal.classList.contains('open') || auditModal.style.display !== 'none')) {
        if (typeof closeAutomationAuditModal === 'function') closeAutomationAuditModal();
      }
    }
  });

  // Background polling intervals
  setInterval(fetchVaultStatus, 5000);
  setInterval(updateAvailableBalanceLabel, 3000);
  setTimeout(fetchVaultStatus, 400);
  setTimeout(updateScannerTelemetry, 300);
  setTimeout(fetchMempoolData, 400);
  setTimeout(updateAvailableBalanceLabel, 500);
});
