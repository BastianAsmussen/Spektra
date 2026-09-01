---
title: "Distribueret overvågning af radiosignalkvalitet"
author: "Bastian Almar Wolsgaard Asmussen"
lang: da-DK
---

## Case beskrivelse

Radio er kritisk infrastruktur og bruges blandt andet til beredskabsinformation,
når andre kommunikationskanaler er nede. Alligevel opdages et forringet signal
typisk først, når lytterne klager, eller ved en manuel måling med fast interval.
En fejl, der udvikler sig langsomt, får ikke signalet til at forsvinde. Det
bliver blot gradvist dårligere, og en gradvis forværring er præcis den type fejl,
et menneske er dårligst til at opdage.

Samtidig står der mange billige radiomodtagere rundt omkring hos private og hos
institutioner. Den enkelte modtager er upålidelig alene, men mange modtagere set
samlet giver et brugbart billede af, hvordan et signal faktisk opfattes. Der
mangler et fælles, åbent grundlag for, at de kan bidrage, og løbende måling fra
mange modtagere giver en datamængde, som skal struktureres derefter.

Indsamling alene er ikke nok. En automatisk opdagelse er kun en formodning,
indtil et menneske har efterprøvet den på stedet, og resultatet skal tilbageføres
til systemet. Ellers kan det aldrig afgøres, hvilke alarmer der var reelle.

## Problemformulering

Hvordan kan man lave en "plug-and-play"-løsning til eksisterende radiomodtagere,
der rapporterer signalkvalitet til en central server, som automatisk opdager
degraderede signaler, visualiserer datagrundlaget og understøtter, at fejlen
lukkes af et menneske fysisk nær radiomodtageren?
