// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
'use strict';

(() => {
  const boards = {
    am62p: { name: 'Verdin AM62P', target: 'verdin_am62p/am62p54/a53' },
    imx8mp: { name: 'Verdin iMX8M Plus', target: 'verdin_imx8mp/mimx8ml8/a53' },
    pi4: { name: 'Raspberry Pi 4', target: 'rpi_4b' }
  };
  const picker = document.getElementById('board-select');
  if (!picker) return;

  const instructions = document.querySelectorAll('[data-board-only]');
  const buildTargets = document.querySelectorAll('[data-build-board]');
  const status = document.getElementById('board-selection');

  function showBoard(value, updateUrl) {
    const board = Object.prototype.hasOwnProperty.call(boards, value) ? value : 'all';
    picker.value = board;
    for (const section of instructions) {
      section.hidden = board !== 'all' && !section.dataset.boardOnly.split(/\s+/).includes(board);
    }
    for (const target of buildTargets) {
      target.textContent = board === 'all' ? '<board>' : boards[board].target;
    }
    if (status) {
      status.textContent = board === 'all'
        ? 'All three board guides are shown below.'
        : `Showing ${boards[board].name} prerequisites and boot instructions, with the common build steps.`;
    }
    if (updateUrl) {
      const url = new URL(window.location.href);
      if (board === 'all') url.searchParams.delete('board');
      else url.searchParams.set('board', board);
      url.hash = 'board';
      window.history.replaceState(null, '', url);
    }
  }

  function readSelection() {
    showBoard(new URLSearchParams(window.location.search).get('board'), false);
  }

  readSelection();
  picker.disabled = false;
  picker.addEventListener('change', () => showBoard(picker.value, true));
  window.addEventListener('popstate', readSelection);
})();
