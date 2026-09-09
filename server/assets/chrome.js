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

  const wanted = new URLSearchParams(location.search).get("state");
  if (stateFilter && wanted &&
      [...stateFilter.options].some((option) => option.value === wanted)) {
    stateFilter.value = wanted;
  }

  function tileState(tile) {
    return tile.querySelector("[data-state]")?.dataset.state ?? "";
  }

  function applyFilter() {
    if (!fleet) {
      return;
    }

    const term = (search?.value ?? "").trim().toLowerCase();
    const wanted = stateFilter?.value ?? "";
    const visible = [];

    for (const tile of fleet.querySelectorAll("[data-node]")) {
      const name = tile.querySelector("h3")?.textContent?.toLowerCase() ?? "";
      const matches =
        (term === "" || name.includes(term)) &&
        (wanted === "" || tileState(tile) === wanted);

      tile.hidden = !matches;
      if (matches) {
        visible.push(Number(tile.dataset.node));
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

  function recount() {
    if (!fleet) {
      return;
    }

    const tiles = [...document.querySelectorAll("#fleet [data-node]")];
    const alarms = [...document.querySelectorAll("#alarm-feed [id^='alarm-']")];
    const orders = [...document.querySelectorAll("#work-orders [data-status]")];

    set("count-nodes", tiles.length, false);
    set(
      "count-silent",
      tiles.filter((tile) => tileState(tile) === "silent").length,
      true,
      "text-yellow",
    );
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

  document.body.addEventListener("htmx:afterSwap", recount);
  document.body.addEventListener("htmx:oobAfterSwap", recount);
  document.addEventListener("DOMContentLoaded", () => {
    applyFilter();
    recount();
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
