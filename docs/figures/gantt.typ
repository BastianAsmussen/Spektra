// Gantt chart for tidsplan.mmd and realiseret.mmd, fed as JSON by build.sh.
// A chart with any `done` task colours its bars by status. Without one, every bar is a plan bar.

#let data = json(sys.inputs.data)

#let ink = rgb("#4c4f69")
#let faint = rgb("#6c6f85")
#let rule = rgb("#ccd0da")
#let weekend = rgb("#eff1f5")
#let blue = (fill: rgb("#bfd1fb"), stroke: rgb("#1e66f5"))
#let peach = (fill: rgb("#ffcfb2"), stroke: rgb("#fe640b"))

#let label-w = 8.8cm
#let chart-w = 14.6cm
#let row-h = 0.56cm
#let bar-h = 0.36cm

#set page(width: label-w + chart-w + 0.6cm, height: auto, margin: 0.3cm, fill: white)
#set text(font: "Libertinus Serif", size: 10pt, fill: ink, lang: "da")

#let parse(s) = {
  let p = s.split("-").map(int)
  datetime(year: p.at(0), month: p.at(1), day: p.at(2))
}

#let tasks = data.sections.map(s => s.tasks).flatten()
#let first = tasks.map(t => parse(t.start)).sorted().first()
#let start = first - duration(days: first.weekday() - 1)
#let end = tasks.map(t => parse(t.start) + duration(days: calc.max(t.days, 1))).sorted().last()
#let ndays = int((end - start).days())
#let day-w = chart-w / ndays
#let status = tasks.any(t => "done" in t.tags)

#let track(body) = box(width: chart-w, height: row-h, body)

#let grid-bg = {
  let h = (2 + data.sections.len() + tasks.len()) * row-h
  for i in range(ndays) {
    let d = start + duration(days: i)
    if d.weekday() >= 6 {
      place(dx: label-w + i * day-w, rect(width: day-w, height: h, fill: weekend, stroke: none))
    }
    if d.weekday() == 1 {
      place(dx: label-w + i * day-w, line(length: h, angle: 90deg, stroke: 0.5pt + rule))
    }
  }
}

#let row(label, body, strong: false) = block(spacing: 0pt, stack(dir: ltr,
  box(width: label-w, height: row-h, inset: (left: if strong { 0pt } else { 0.3cm }),
    align(horizon, if strong { text(weight: "bold", label) } else { label })),
  track(body),
))

#let style(t) = if not status or "done" in t.tags { blue } else if "active" in t.tags { peach } else {
  (fill: white, stroke: blue.stroke)
}

#let bar(t) = {
  let off = (parse(t.start) - start).days() * day-w
  if "milestone" in t.tags {
    let s = bar-h * 1.1
    place(dx: off - s / 2, dy: (row-h - s) / 2,
      rotate(45deg, rect(width: s * 0.72, height: s * 0.72, fill: ink, stroke: none)))
  } else {
    let st = style(t)
    let dashed = status and not ("done" in t.tags or "active" in t.tags)
    place(dx: off, dy: (row-h - bar-h) / 2, rect(width: t.days * day-w, height: bar-h, radius: 2pt,
      fill: st.fill, stroke: (paint: st.stroke, thickness: 0.8pt, dash: if dashed { "dashed" } else { none })))
  }
}

#let axis = {
  let weeks = range(ndays).filter(i => (start + duration(days: i)).weekday() == 1)
  row([], {
    for i in weeks {
      let d = start + duration(days: i)
      place(dx: i * day-w + 2pt, dy: 1pt, text(size: 9pt, weight: "bold", "Uge " + d.display("[week_number repr:iso]")))
    }
  })
  row([], {
    for i in range(ndays) {
      let d = start + duration(days: i)
      place(dx: i * day-w, dy: 2pt, box(width: day-w, align(center, text(size: 7.5pt, fill: faint,
        if d.day() == 1 or i == 0 { d.display("[day padding:none]/[month padding:none]") } else { str(d.day()) }))))
    }
  })
}

#grid-bg
#axis
#for s in data.sections {
  block(spacing: 0pt, line(length: 100%, stroke: 0.5pt + rule))
  row(s.name, [], strong: true)
  for t in s.tasks { row(t.label, bar(t)) }
}
#block(spacing: 0pt, line(length: 100%, stroke: 0.5pt + rule))

#if status {
  v(0.25cm)
  let key(st, dashed, name) = box(baseline: 20%, rect(width: 0.9cm, height: bar-h, radius: 2pt, fill: st.fill,
    stroke: (paint: st.stroke, thickness: 0.8pt, dash: if dashed { "dashed" } else { none }))) + h(0.2cm) + name
  align(right, stack(dir: ltr, spacing: 0.6cm,
    key(blue, false, "Udført"),
    key(peach, false, "I gang"),
    key((fill: white, stroke: blue.stroke), true, "Ikke påbegyndt"),
  ))
}
