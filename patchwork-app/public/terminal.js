(function () {
  const terminals = new Map();
  const POLL_INTERVAL_MS = 180;
  let nextTerminalId = 1;

  function tauriInvoke(command, args) {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) {
      return Promise.resolve(null);
    }
    return invoke(command, args).catch((error) => {
      console.error(`Patchwork terminal command failed: ${command}`, error);
      return null;
    });
  }

  function decodeBase64(base64) {
    const binary = window.atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return bytes;
  }

  function encodeBytes(bytes) {
    let binary = "";
    const chunkSize = 0x8000;
    for (let offset = 0; offset < bytes.length; offset += chunkSize) {
      binary += String.fromCharCode.apply(
        null,
        bytes.subarray(offset, offset + chunkSize),
      );
    }
    return window.btoa(binary);
  }

  function encodeTerminalInput(data) {
    return encodeBytes(new TextEncoder().encode(data));
  }

  function encodeBinaryInput(data) {
    const bytes = new Uint8Array(data.length);
    for (let index = 0; index < data.length; index += 1) {
      bytes[index] = data.charCodeAt(index) & 0xff;
    }
    return encodeBytes(bytes);
  }

  function terminalTheme() {
    const shell = document.querySelector(".app-shell") || document.documentElement;
    const style = window.getComputedStyle(shell);
    const read = (name, fallback) => style.getPropertyValue(name).trim() || fallback;
    return {
      background: read("--console-bg", "#101219"),
      foreground: read("--ink", "#e9edf6"),
      cursor: read("--accent", "#02a9a9"),
      selectionBackground: "rgba(98, 104, 200, 0.35)",
      black: "#111318",
      red: "#fd614e",
      green: "#6ee787",
      yellow: "#fdb22c",
      blue: "#6268c8",
      magenta: "#ff6bd6",
      cyan: "#02a9a9",
      white: "#f4f7fb",
      brightBlack: "#656b78",
      brightRed: "#ff8a7b",
      brightGreen: "#95f5ad",
      brightYellow: "#ffd06a",
      brightBlue: "#8a90ff",
      brightMagenta: "#ff95e2",
      brightCyan: "#62eeee",
      brightWhite: "#ffffff",
    };
  }

  function fitAndNotify(state) {
    if (!state || state.disposed) {
      return;
    }
    try {
      state.fitAddon.fit();
      const rows = state.terminal.rows;
      const cols = state.terminal.cols;
      if (!state.profileId || rows < 1 || cols < 2) {
        return;
      }
      if (state.lastRows === rows && state.lastCols === cols) {
        return;
      }
      state.lastRows = rows;
      state.lastCols = cols;
      tauriInvoke("resize_patchwork_terminal", {
        profileId: state.profileId,
        rows,
        cols,
      });
    } catch (error) {
      console.error("Patchwork terminal fit failed", error);
    }
  }

  function scheduleFit(state) {
    if (state.fitFrame) {
      cancelAnimationFrame(state.fitFrame);
    }
    state.fitFrame = requestAnimationFrame(() => {
      state.fitFrame = 0;
      fitAndNotify(state);
    });
  }

  async function pollTerminalOutput(state) {
    if (!state || state.disposed || state.polling || !state.profileId) {
      return;
    }

    state.polling = true;
    const profileId = state.profileId;
    const offset = state.remoteOffset;
    try {
      const chunk = await tauriInvoke("patchwork_console_chunk", {
        profileId,
        offset,
      });
      if (!chunk || state.disposed || state.profileId !== profileId) {
        return;
      }

      if (chunk.reset) {
        resetTerminal(state);
        state.remoteOffset = Number(chunk.startOffset || 0);
      }
      if (chunk.bytes) {
        writeTerminalBytes(state, chunk.bytes);
      }
      state.remoteOffset = Number(chunk.endOffset || state.remoteOffset);
    } finally {
      state.polling = false;
    }
  }

  function startPolling(state) {
    if (state.pollInterval) {
      clearInterval(state.pollInterval);
    }
    state.pollInterval = setInterval(
      () => pollTerminalOutput(state),
      POLL_INTERVAL_MS,
    );
    pollTerminalOutput(state);
  }

  window.patchworkCreateTerminal = function (element, profileId, buildMode) {
    if (!window.Terminal || !window.FitAddon?.FitAddon) {
      console.error("xterm.js or FitAddon is not loaded");
      return null;
    }

    const id = nextTerminalId;
    nextTerminalId += 1;

    const terminal = new window.Terminal({
      allowTransparency: true,
      convertEol: false,
      cursorBlink: false,
      cursorInactiveStyle: "none",
      cursorStyle: "block",
      fontFamily:
        "JetBrains Mono, ui-monospace, SFMono-Regular, Consolas, Liberation Mono, monospace",
      fontSize: 13,
      lineHeight: 1.25,
      rows: 24,
      cols: 100,
      scrollback: 6000,
      theme: terminalTheme(),
    });
    const fitAddon = new window.FitAddon.FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(element);

    const state = {
      id,
      terminal,
      fitAddon,
      profileId: profileId || "",
      buildMode: buildMode || "release",
      lastRows: 0,
      lastCols: 0,
      resizeObserver: null,
      fitFrame: 0,
      pollInterval: 0,
      polling: false,
      remoteOffset: 0,
      bytesWritten: 0,
      disposed: false,
    };

    terminal.onData((data) => {
      if (!state.profileId) {
        return;
      }
      tauriInvoke("write_patchwork_terminal", {
        profileId: state.profileId,
        data: encodeTerminalInput(data),
      });
    });
    terminal.onBinary((data) => {
      if (!state.profileId) {
        return;
      }
      tauriInvoke("write_patchwork_terminal", {
        profileId: state.profileId,
        data: encodeBinaryInput(data),
      });
    });

    if (window.ResizeObserver) {
      state.resizeObserver = new ResizeObserver(() => scheduleFit(state));
      state.resizeObserver.observe(element);
      if (element.parentElement) {
        state.resizeObserver.observe(element.parentElement);
      }
    } else {
      state.resizeListener = () => scheduleFit(state);
      window.addEventListener("resize", state.resizeListener);
    }

    terminals.set(id, state);
    startPolling(state);
    scheduleFit(state);
    setTimeout(() => scheduleFit(state), 50);
    return id;
  };

  window.patchworkSetTerminalProfile = function (handle, profileId, buildMode) {
    const state = terminals.get(Number(handle));
    if (!state) {
      return;
    }
    state.profileId = profileId || "";
    state.buildMode = buildMode || "release";
    state.lastRows = 0;
    state.lastCols = 0;
    state.polling = false;
    state.remoteOffset = 0;
    resetTerminal(state);
    pollTerminalOutput(state);
    scheduleFit(state);
  };

  window.patchworkResetTerminal = function (handle) {
    const state = terminals.get(Number(handle));
    if (state) {
      resetTerminal(state);
    }
  };

  window.patchworkWriteTerminalBytes = function (handle, bytesBase64) {
    const state = terminals.get(Number(handle));
    if (state && bytesBase64) {
      writeTerminalBytes(state, bytesBase64);
    }
  };

  window.patchworkLoadTerminalSnapshot = function (handle, bytesBase64) {
    const state = terminals.get(Number(handle));
    if (state && bytesBase64) {
      resetTerminal(state);
      writeTerminalBytes(state, bytesBase64);
    }
  };

  window.patchworkFitTerminal = function (handle) {
    const state = terminals.get(Number(handle));
    if (state) {
      scheduleFit(state);
    }
  };

  window.patchworkDisposeTerminal = function (handle) {
    const id = Number(handle);
    const state = terminals.get(id);
    if (!state) {
      return;
    }
    state.disposed = true;
    if (state.fitFrame) {
      cancelAnimationFrame(state.fitFrame);
    }
    if (state.resizeObserver) {
      state.resizeObserver.disconnect();
    }
    if (state.resizeListener) {
      window.removeEventListener("resize", state.resizeListener);
    }
    if (state.pollInterval) {
      clearInterval(state.pollInterval);
    }
    state.terminal.dispose();
    terminals.delete(id);
  };

  function resetTerminal(state) {
    state.terminal.reset();
    state.bytesWritten = 0;
  }

  function writeTerminalBytes(state, bytesBase64) {
    const bytes = decodeBase64(bytesBase64);
    state.bytesWritten += bytes.length;
    state.terminal.write(bytes, () => {
      state.terminal.refresh(0, Math.max(0, state.terminal.rows - 1));
    });
  }
})();
