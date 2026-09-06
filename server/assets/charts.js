(() => {
  "use strict";

  const drawn = new WeakMap();

  const HHMM = new Intl.DateTimeFormat("da-DK", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });

  const DAY = new Intl.DateTimeFormat("da-DK", { day: "numeric", month: "short" });
  const DAY_HHMM = new Intl.DateTimeFormat("da-DK", {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });

  const FULL = new Intl.DateTimeFormat("da-DK", {
    day: "2-digit",
    month: "2-digit",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });

  const DAY_SECONDS = 86_400;

  function timeAxis(splits) {
    const span = splits.length > 1 ? splits[splits.length - 1] - splits[0] : 0;
    const format = span > 7 * DAY_SECONDS ? DAY : span > DAY_SECONDS ? DAY_HHMM : HHMM;

    return splits.map((seconds) => format.format(new Date(seconds * 1000)));
  }

  function decimalsFor(ticks) {
    const steps = ticks
      .slice(1)
      .map((value, index) => Math.abs(value - ticks[index]))
      .filter((step) => step > 0);

    if (steps.length === 0) {
      return 1;
    }

    return Math.min(6, Math.max(0, Math.ceil(-Math.log10(Math.min(...steps)))));
  }

  function palette() {
    const style = getComputedStyle(document.documentElement);
    const read = (name, fallback) => style.getPropertyValue(name).trim() || fallback;

    return {
      line: read("--ctp-blue", "#1e66f5"),
      band: read("--ctp-green", "#40a02b"),
      grid: read("--ctp-surface0", "#ccd0da"),
      axis: read("--ctp-overlay0", "#9ca0b0"),
    };
  }

  function bandHook(band, colors) {
    return (plot) => {
      if (!band) {
        return;
      }

      const [low, high] = band;
      const top = plot.valToPos(Math.max(low, high), "y", true);
      const bottom = plot.valToPos(Math.min(low, high), "y", true);

      const context = plot.ctx;
      context.save();
      context.beginPath();
      context.rect(plot.bbox.left, plot.bbox.top, plot.bbox.width, plot.bbox.height);
      context.clip();
      context.fillStyle = colors.band;
      context.globalAlpha = 0.12;
      context.fillRect(plot.bbox.left, top, plot.bbox.width, Math.max(bottom - top, 0));
      context.globalAlpha = 0.5;
      context.strokeStyle = colors.band;
      context.setLineDash([4, 4]);
      context.beginPath();

      const center = plot.valToPos((low + high) / 2, "y", true);
      context.moveTo(plot.bbox.left, center);
      context.lineTo(plot.bbox.left + plot.bbox.width, center);
      context.stroke();
      context.restore();
    };
  }

  function build(figure, target, series) {
    const colors = palette();
    const count = series.points.at.length;

    target.replaceChildren();

    if (count < 3) {
      target.textContent =
        count === 0 ? "Ingen målinger i perioden." : `For få målinger til en graf (${count}).`;

      return null;
    }

    const decimals = series.presentation.unit === "%" ? 2 : 1;
    const reset = figure.querySelector("[data-chart-reset]");
    const full = [series.points.at[0], series.points.at[count - 1]];

    const zoomHook = (plot) => {
      if (!reset) {
        return;
      }

      const { min, max } = plot.scales.x;
      reset.hidden = min <= full[0] && max >= full[1];
    };

    const plot = new uPlot(
      {
        width: target.clientWidth,
        height: target.clientHeight,
        padding: [8, 8, 0, 0],
        cursor: { y: false, points: { size: 6 } },
        legend: { show: false },
        scales: { x: { time: true } },
        axes: [
          {
            stroke: colors.axis,
            grid: { stroke: colors.grid, width: 1 },
            ticks: { stroke: colors.grid },
            font: "11px ui-monospace, monospace",
            space: 96,
            values: (_plot, splits) => timeAxis(splits),
          },
          {
            stroke: colors.axis,
            grid: { stroke: colors.grid, width: 1 },
            ticks: { stroke: colors.grid },
            font: "11px ui-monospace, monospace",
            size: 62,
            values: (_plot, ticks) => {
              const places = Math.max(decimals, decimalsFor(ticks));

              return ticks.map((tick) => tick.toFixed(places));
            },
          },
        ],
        series: [
          {
            value: (_plot, raw) => (raw == null ? "" : FULL.format(new Date(raw * 1000))),
          },
          {
            label: series.presentation.name,
            stroke: colors.line,
            width: 1.5,
            points: { show: count < 120 },
            value: (_plot, raw) =>
              raw == null ? "" : `${raw.toFixed(decimals)} ${series.presentation.unit}`,
          },
        ],
        hooks: { draw: [bandHook(series.band, colors)], setScale: [zoomHook] },
      },
      [series.points.at, series.points.value],
      target,
    );

    if (reset) {
      reset.hidden = true;
      reset.onclick = () => {
        plot.setScale("x", { min: full[0], max: full[1] });
      };
    }

    return plot;
  }

  async function load(figure) {
    const url = figure.dataset.series;
    const target = figure.querySelector("[data-chart]");
    if (!url || !target) {
      return;
    }

    let series;
    try {
      const response = await fetch(url, { credentials: "same-origin" });
      if (!response.ok) {
        target.textContent = `Kunne ikke hente data (${response.status}).`;

        return;
      }

      series = await response.json();
    } catch {
      target.textContent = "Kunne ikke hente data.";

      return;
    }

    const name = figure.querySelector("[data-chart-name]");
    if (name) {
      name.textContent = `${series.presentation.name} (${series.presentation.unit})`;
    }

    const source = figure.querySelector("[data-chart-source]");
    if (source) {
      source.textContent = series.source_name;
    }

    const plot = build(figure, target, series);
    if (plot) {
      drawn.set(figure, { plot, series });
    }
  }

  const resizes = new ResizeObserver((entries) => {
    for (const entry of entries) {
      const state = drawn.get(entry.target);
      if (state) {
        const target = entry.target.querySelector("[data-chart]");
        if (target && target.clientWidth > 0) {
          state.plot.setSize({ width: target.clientWidth, height: target.clientHeight });
        }
      }
    }
  });

  const hidden = (figure) => figure.closest("details:not([open])") !== null;

  function scan(root) {
    const figures =
      root instanceof Element && root.matches?.("figure[data-series]")
        ? [root]
        : (root.querySelectorAll?.("figure[data-series]") ?? []);

    for (const figure of figures) {
      if (!drawn.has(figure) && !figure.dataset.chartPending && !hidden(figure)) {
        figure.dataset.chartPending = "1";
        resizes.observe(figure);
        load(figure).finally(() => delete figure.dataset.chartPending);
      }
    }
  }

  document.body.addEventListener("htmx:afterSwap", (event) => scan(event.detail.target));
  document.addEventListener("DOMContentLoaded", () => scan(document));
  document.addEventListener(
    "toggle",
    (event) => {
      if (event.target instanceof HTMLDetailsElement && event.target.open) {
        scan(event.target);
      }
    },
    true,
  );

  window.addEventListener("spektra:theme", () => {
    for (const figure of document.querySelectorAll("figure[data-series]")) {
      const state = drawn.get(figure);
      if (state) {
        state.plot.destroy();
        drawn.delete(figure);
      }
    }

    scan(document);
  });
})();
