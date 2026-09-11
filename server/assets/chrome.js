(() => {
  "use strict";

  const THEME_KEY = "spektra-theme";

  const themeButtons = [...document.querySelectorAll("[data-theme-choice]")];

  function currentTheme() {
    return document.documentElement.dataset.theme || "system";
  }

  function applyTheme(choice) {
    if (choice === "system") {
      delete document.documentElement.dataset.theme;
    } else {
      document.documentElement.dataset.theme = choice;
    }

    try {
      if (choice === "system") {
        localStorage.removeItem(THEME_KEY);
      } else {
        localStorage.setItem(THEME_KEY, choice);
      }
    } catch {
    }

    for (const button of themeButtons) {
      button.setAttribute(
        "aria-pressed",
        String(button.dataset.themeChoice === choice),
      );
    }

    window.dispatchEvent(new CustomEvent("spektra:theme", { detail: { choice } }));
  }

  for (const button of themeButtons) {
    button.addEventListener("click", () => applyTheme(button.dataset.themeChoice));
  }

  applyTheme(currentTheme());

  const menuWrap = document.getElementById("menu-wrap");
  if (menuWrap) {
    const close = () => menuWrap.removeAttribute("open");

    document.addEventListener("click", (event) => {
      if (!menuWrap.contains(event.target)) {
        close();
      }
    });
    document.addEventListener("keydown", (event) => event.key === "Escape" && close());

    menuWrap.addEventListener("click", (event) => event.target.closest("a") && close());
  }

  const search = document.getElementById("fleet-search");
  const stateFilter = document.getElementById("fleet-state");
  const fleet = document.getElementById("fleet");

  let roster = [];
  try {
    roster = JSON.parse(document.getElementById("fleet-data")?.textContent ?? "[]");
  } catch (error) {
    console.error("fleet data is not valid JSON", error);
  }

  window.spektraFleet = roster;

  const states = new Map(roster.map((node) => [node.id, node.state]));
  let silent = roster.reduce((total, node) => total + (node.state === "silent" ? 1 : 0), 0);

  const wanted = new URLSearchParams(location.search).get("state");
  if (stateFilter && wanted &&
      [...stateFilter.options].some((option) => option.value === wanted)) {
    stateFilter.value = wanted;
  }

  function applyFilter() {
    if (!fleet) {
      return;
    }

    const term = (search?.value ?? "").trim().toLowerCase();
    const wanted = stateFilter?.value ?? "";
    const visible = [];

    for (const node of roster) {
      const matches =
        (term === "" || node.name.toLowerCase().includes(term)) &&
        (wanted === "" || states.get(node.id) === wanted);

      if (matches) {
        visible.push(node.id);
      }
    }

    window.dispatchEvent(
      new CustomEvent("spektra:filter", { detail: { visible } }),
    );
  }

  search?.addEventListener("input", applyFilter);
  stateFilter?.addEventListener("change", applyFilter);
  document.getElementById("fleet-refit")?.addEventListener("click", () => {
    window.dispatchEvent(new CustomEvent("spektra:refit"));
  });

  let pendingPaint = 0;
  function paintNodeCounts() {
    if (!fleet || pendingPaint) {
      return;
    }

    pendingPaint = requestAnimationFrame(() => {
      pendingPaint = 0;
      set("count-nodes", roster.length, false);
      set("count-silent", silent, true, "text-yellow");
    });
  }

  function recountFeeds() {
    if (!fleet) {
      return;
    }

    const alarms = [...document.querySelectorAll("#alarm-feed [id^='alarm-']")];
    const orders = [...document.querySelectorAll("#work-orders [data-status]")];

    set(
      "count-alarms",
      alarms.filter((alarm) => alarm.dataset.state && alarm.dataset.state !== "closed").length,
      true,
      "text-red",
    );
    set(
      "count-orders",
      orders.filter((order) => order.dataset.status === "assigned").length,
      true,
      "text-blue",
    );
  }

  function observeState(element) {
    const match = /^node-(\d+)-state$/.exec(element.id ?? "");
    if (!match) {
      return false;
    }

    const id = Number(match[1]);
    const before = states.get(id);
    const after = element.dataset.state ?? "never seen";
    if (before === after) {
      return true;
    }

    states.set(id, after);
    if (before === "silent") {
      silent -= 1;
    }
    if (after === "silent") {
      silent += 1;
    }
    paintNodeCounts();

    return true;
  }

  function set(id, value, colorWhenNonZero, color) {
    const element = document.getElementById(id);
    if (!element) {
      return;
    }

    element.textContent = String(value);
    if (colorWhenNonZero) {
      element.className = `font-mono tabular-nums ${value > 0 ? color : "text-overlay0"}`;
    }
  }

  document.body.addEventListener("htmx:oobAfterSwap", (event) => {
    if (!observeState(event.detail.target)) {
      recountFeeds();
    }
  });
  document.body.addEventListener("htmx:afterSwap", recountFeeds);
  document.addEventListener("DOMContentLoaded", () => {
    applyFilter();
    paintNodeCounts();
    recountFeeds();
  });

  const clockParts = new Intl.DateTimeFormat("da-DK", {
    timeZone: "Europe/Copenhagen",
    hour12: false,
    weekday: "short",
    day: "2-digit",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });

  const clockText = (at) => {
    const p = Object.fromEntries(
      clockParts.formatToParts(at).map((part) => [part.type, part.value]),
    );

    const bare = (value) => value.replace(/\.$/, "");

    return `${bare(p.weekday)} ${p.day}. ${bare(p.month)} ${p.year} - ${p.hour}:${p.minute}:${p.second}`;
  };

  const clock = document.getElementById("clock");
  if (clock) {
    const text = document.createTextNode("");
    clock.replaceChildren(text);

    const tick = () => {
      const now = new Date();
      text.nodeValue = clockText(now).toUpperCase();
      clock.dateTime = now.toISOString();
    };

    tick();
    setInterval(tick, 1000);
  }

  const status = document.getElementById("live-status");
  const statusText = status?.querySelector("[data-live-text]");
  const setStatus = (text, color) => {
    if (!status || !statusText) {
      return;
    }

    statusText.textContent = text;
    status.className = `hidden items-center gap-1.5 font-mono text-xs sm:flex ${color}`;
  };

  document.body.addEventListener("htmx:wsOpen", () => setStatus("forbundet", "text-green"));
  document.body.addEventListener("htmx:wsClose", (event) => {
    if (event.detail.event?.reason === "session-expired") {
      window.location.assign("/login");

      return;
    }

    setStatus("afbrudt", "text-yellow");
  });
  document.body.addEventListener("htmx:wsError", () => setStatus("ikke logget ind", "text-red"));
})();
