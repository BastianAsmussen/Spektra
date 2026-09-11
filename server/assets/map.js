(() => {
  "use strict";

  const DENMARK = [56.15, 10.2];

  const DEFAULT_ZOOM = 7;
  const FOCUS_ZOOM = 9;

  const STATES = ["reporting", "silent", "suspended", "never-seen"];
  const slug = (state) => String(state).trim().replace(/[^a-z]+/g, "-");

  const element = document.getElementById("map");
  const payload = document.getElementById("fleet-data");
  if (!element || !payload || typeof L === "undefined") {
    return;
  }

  let fleet = window.spektraFleet;
  if (!Array.isArray(fleet)) {
    try {
      fleet = JSON.parse(payload.textContent);
    } catch (error) {
      console.error("fleet data is not valid JSON", error);

      return;
    }
  }

  const placed = fleet.filter(
    (node) => typeof node.latitude === "number" && typeof node.longitude === "number",
  );

  const unplaced = document.getElementById("unplaced");
  if (unplaced) {
    const missing = fleet.length - placed.length;
    if (missing > 0) {
      unplaced.textContent =
        missing === 1
          ? "1 node har ingen registreret position og vises ikke på kortet."
          : `${missing} noder har ingen registreret position og vises ikke på kortet.`;
    } else {
      unplaced.remove();
    }
  }

  const map = L.map(element, {
    center: DENMARK,
    zoom: DEFAULT_ZOOM,
    scrollWheelZoom: false,
  });

  const hint = L.DomUtil.create("div", "map-hint", element);
  hint.textContent = "Hold Shift for at zoome";

  let hintTimer = 0;

  element.addEventListener(
    "wheel",
    (event) => {
      if (!event.shiftKey) {
        hint.classList.add("map-hint--on");

        clearTimeout(hintTimer);
        hintTimer = setTimeout(() => hint.classList.remove("map-hint--on"), 1400);

        return;
      }

      event.preventDefault();
      hint.classList.remove("map-hint--on");
      map.setZoomAround(
        map.mouseEventToLatLng(event),
        map.getZoom() + (event.deltaY < 0 ? 1 : -1),
      );
    },
    { passive: false },
  );

  new ResizeObserver(() => map.invalidateSize()).observe(element);

  L.tileLayer("https://tile.openstreetmap.org/{z}/{x}/{y}.png", {
    maxZoom: 18,
    attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a>',
  }).addTo(map);

  if (placed.length > 0) {
    map.fitBounds(L.latLngBounds(placed.map((node) => [node.latitude, node.longitude])), {
      padding: [32, 32],
      maxZoom: 11,
      animate: false,
    });
  }

  const markers = new Map();
  const rings = new Map();
  let selected = null;
  let selectedId = null;

  function paint(marker, state) {
    const path = marker.getElement();
    if (!path) {
      return;
    }

    for (const known of STATES) {
      path.classList.remove(`marker--${known}`);
    }

    path.classList.add(`marker--${slug(state)}`);
  }

  for (const node of placed) {
    const at = [node.latitude, node.longitude];

    if (node.open_alarms > 0) {
      rings.set(
        node.id,
        L.circleMarker(at, {
          radius: 16,
          className: "marker-ring",
          interactive: false,
        }).addTo(map),
      );
    }

    const marker = L.circleMarker(at, { radius: 7, className: "marker" }).addTo(map);
    paint(marker, node.state);

    marker.bindTooltip(node.name, { direction: "top", offset: [0, -8] });
    marker.on("click", () => select(node.id));

    markers.set(node.id, marker);
  }

  function options(search = location.search) {
    const from = new URLSearchParams(search);
    const to = new URLSearchParams();
    for (const key of ["span", "metrics"]) {
      const value = from.get(key);
      if (value) {
        to.set(key, value);
      }
    }

    const query = to.toString();

    return query ? `?${query}` : "";
  }

  function mark(nodeId, pan) {
    const marker = markers.get(nodeId);

    selected?.getElement()?.classList.remove("marker--selected");
    selected = marker ?? null;
    selectedId = marker ? nodeId : null;

    marker?.getElement()?.classList.add("marker--selected");
    if (marker && pan) {
      map.setView(marker.getLatLng(), Math.max(map.getZoom(), FOCUS_ZOOM));
    }
  }

  history.scrollRestoration = "manual";

  function rememberScroll() {
    history.replaceState({ ...history.state, scrollY: window.scrollY }, "");
  }

  function afterPanel(run) {
    const panel = document.getElementById("node-panel");
    if (!panel) {
      run();

      return;
    }

    panel.addEventListener("htmx:afterSettle", () => requestAnimationFrame(run), { once: true });
  }

  function select(nodeId, { push = true, pan = true, search = location.search, scrollY } = {}) {
    mark(nodeId, pan);

    const panel = document.getElementById("node-panel");
    if (panel && window.htmx) {
      afterPanel(() => {
        if (typeof scrollY === "number") {
          window.scrollTo({ top: scrollY, behavior: "instant" });
        } else {
          panel.scrollIntoView({ block: "start", behavior: "instant" });
        }
      });

      window.htmx.ajax("GET", `/fragments/nodes/${nodeId}${options(search)}`, {
        target: panel,
        swap: "innerHTML",
      });
    }

    if (push) {
      rememberScroll();

      history.pushState({ nodeId }, "", `/nodes/${nodeId}${options(search)}`);
    }
  }

  function deselect({ push = true, scrollY } = {}) {
    selected?.getElement()?.classList.remove("marker--selected");
    selected = null;
    selectedId = null;

    if (push) {
      rememberScroll();
    }

    document.getElementById("node-panel")?.replaceChildren();

    if (push) {
      history.pushState({}, "", "/");
      window.scrollTo({ top: 0, behavior: "instant" });
    } else if (typeof scrollY === "number") {
      requestAnimationFrame(() => window.scrollTo({ top: scrollY, behavior: "instant" }));
    }
  }

  document.getElementById("fleet")?.addEventListener("click", (event) => {
    const tile = event.target.closest("[data-node]");
    if (tile) {
      select(Number(tile.dataset.node));
    }
  });

  document.getElementById("node-panel")?.addEventListener("click", (event) => {
    if (event.target.closest("[data-close-panel]")) {
      event.preventDefault();
      deselect();

      return;
    }

    const link = event.target.closest("[data-panel-link]");
    if (link?.dataset.url) {
      const here = window.scrollY;
      afterPanel(() => window.scrollTo({ top: here, behavior: "instant" }));

      rememberScroll();
      history.pushState({ nodeId: selectedId }, "", link.dataset.url);
    }
  });

  addEventListener("popstate", (event) => {
    const scrollY = typeof event.state?.scrollY === "number" ? event.state.scrollY : 0;

    const match = /^\/nodes\/(\d+)/.exec(location.pathname);
    if (match) {
      select(Number(match[1]), { push: false, pan: false, scrollY });
    } else {
      deselect({ push: false, scrollY });
    }
  });

  const opened = /^\/nodes\/(\d+)/.exec(location.pathname);
  if (opened) {
    mark(Number(opened[1]), true);
  }

  const shell = document.getElementById("map-shell");
  const fullscreenButton = document.getElementById("map-fullscreen");
  if (shell && fullscreenButton) {
    fullscreenButton.addEventListener("click", () => {
      if (document.fullscreenElement === shell) {
        document.exitFullscreen().catch(() => {});
      } else {
        shell.requestFullscreen().catch(() => {});
      }
    });

    document.addEventListener("fullscreenchange", () => {
      const on = document.fullscreenElement === shell;

      fullscreenButton.setAttribute("aria-pressed", String(on));
      fullscreenButton.title = on ? "Forlad fuld skærm" : "Fuld skærm";
      for (const icon of fullscreenButton.querySelectorAll("[data-when]")) {
        icon.hidden = (icon.dataset.when === "fullscreen") !== on;
      }

      map.invalidateSize();
    });
  }

  addEventListener("spektra:filter", (event) => {
    const visible = new Set(event.detail.visible);
    for (const [nodeId, marker] of markers) {
      const element = marker.getElement();
      if (element) {
        element.style.display = visible.has(nodeId) ? "" : "none";
      }

      const ring = rings.get(nodeId);
      const ringElement = ring?.getElement();
      if (ringElement) {
        ringElement.style.display = visible.has(nodeId) ? "" : "none";
      }
    }
  });

  addEventListener("spektra:refit", () => {
    const shown = [...markers.entries()]
      .filter(([, marker]) => marker.getElement()?.style.display !== "none")
      .map(([, marker]) => marker.getLatLng());

    if (shown.length > 0) {
      map.fitBounds(L.latLngBounds(shown), { padding: [32, 32], maxZoom: 12 });
    }
  });

  const repaint = new Map();
  let pendingPaint = 0;

  document.body.addEventListener("htmx:oobAfterSwap", (event) => {
    const target = event.detail.target;
    const match = /^node-(\d+)-state$/.exec(target.id ?? "");
    if (!match) {
      return;
    }

    repaint.set(Number(match[1]), target.dataset.state ?? "never seen");
    if (pendingPaint) {
      return;
    }

    pendingPaint = requestAnimationFrame(() => {
      pendingPaint = 0;
      for (const [nodeId, state] of repaint) {
        const marker = markers.get(nodeId);
        if (marker) {
          paint(marker, state);
        }
      }

      repaint.clear();
    });
  });
})();
