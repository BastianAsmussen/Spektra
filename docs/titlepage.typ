#let pandoc-conf = conf

#let conf(title: none, subtitle: none, authors: (), date: none, ..args) = pandoc-conf(
  ..(args.named() + (pagenumbering: none)),
  {
    set document(title: title)
    let name = authors.first().name
    let date = "24. september 2026"

    v(1fr)
    align(center)[
      #text(size: 2.2em, weight: "bold", subtitle)
      #v(1em)
      #text(size: 1.4em, title)
      #v(0.5em)
      #text(size: 1.2em)[Spektra]
      #v(3em)
      #image("docs/figures/coverphoto.jpg", width: 100%)
      #v(3em)
      #name \
      #date
    ]
    v(1fr)
    pagebreak()

    let field(label, value) = block(below: 1.6em)[
      #text(size: 1.2em, weight: "bold")[#label:] \
      #value
    ]

    grid(
      columns: (1fr, 1fr),
      column-gutter: 3em,
      align(center)[
        #v(1em)
        #image("docs/figures/techcollege.png", width: 85%)
        #v(2em)
        Techcollege Aalborg, \
        Struervej 70, \
        9220 Aalborg
      ],
      {
        set par(justify: false)
        field("Elev", name)
        field("Firma", "Spektra")
        field("Projekt", "Spektra")
        field("Uddannelse", "Datatekniker med speciale i programmering")
        field("Projektperiode", "31/08/2026 til 24/09/2026")
        field("Afleveringsdato", "24/09/2026")
        field("Fremlæggelsesdato", "01/10/2026")
        field("Vejledere", [Simon Hoxer Bønding, \ Lars Thise Pedersen])
        field("Underskrift", {
          v(2.5em)
          line(length: 100%)
          name
        })
      },
    )

    set page(numbering: args.named().at("pagenumbering", default: "1"))
    counter(page).update(1)
    args.pos().first()
  },
)
