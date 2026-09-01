---
title: "Distribueret overvågning af radiosignalkvalitet"
subtitle: "Produktrapport"
author:
  - name: "Bastian Almar Wolsgaard Asmussen"
    affiliation: '`#text(size: 0.8em)[Vejledere: Simon Hoxer Bønding og Lars Thise Pedersen]`{=typst}'
date: "24. september 2026"
lang: da-DK
---

# Læsevejledning

Rapporten beskriver Spektra som produkt: kravene, afgrænsningen, hvordan det
anvendes, og hvordan det er bygget. Procesrapporten beskriver forløbet og de
valg, der formede produktet.

Rapporten bør læses før procesrapporten, fordi den viser, hvad systemet er.
Procesrapporten viser, hvordan det blev til.

Systemets overordnede arkitektur er vist i bilag 1. Diagrammet viser systemets
fire komponenter og de to grænseflader mellem dem.

Databasens opbygning er vist i bilag 2.

Systemet er publiceret på <https://spektra.asmussen.tech> og kræver ingen
installation for at blive afprøvet. Adressen, administratorens brugernavn og
adgangskode samt en vejledning til brugen af systemet findes i kapitlet
Brugervejledning.

Kildekoden til protokol, server og node-agent findes på
<https://github.com/BastianAsmussen/Spektra>.

Case beskrivelse og problemformulering optræder ordret i begge rapporter.


# Case beskrivelse

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


# Krav- og accepttestspecifikation


## Definition af produktet

**Node-agenten** afvikles på en Linux-baseret enkeltkortscomputer med en
tilsluttet SDR-modtager. Agenten sampler radiospektret, udleder
kvalitetsmetrikker fra det rå IQ-signal, aggregerer lokalt og rapporterer til
den centrale server.

**Den centrale server** modtager, validerer og persisterer måledata,
vedligeholder en baseline pr. node og kanal, detekterer afvigelser, administrerer
alarmer og de arbejdsordrer de giver anledning til, udsender notifikationer,
overvåger sin egen drift og eksponerer et autentificeret API.

**Webklienten** er systemets operative grænseflade: visualisering af måledata,
håndtering af alarmer og styring af udkald.

**Protokolspecifikationen** er en selvstændig, versioneret leverance, der
beskriver dataformat, transport, autentificering og versioneringsregler i et
maskinlæsbart skema. Specifikationen er offentlig, så en tredjepart kan
implementere en node uden adgang til nodeagentens kildekode.


## Kategorier

| ID | Kategori | Beskrivelse |
|:------|:------------------|:----------------------------------------------------------------------------|
| KN1 | Acceptance | Krav der på højere niveau fastsætter, hvad brugeren skal kunne, for at systemet løser det problem, som er beskrevet i problemformuleringen. |
| KN2 | Funktionalitet | Rå funktionalitet, som bruges direkte, eller som andre krav afhænger af. |
| KN3 | Sikkerhed | Krav der beskytter datagrundlaget og systemet mod defekte, fejlkonfigurerede eller ondsindede noder og brugere. |
| KN4 | Drift | Krav til systemets egen driftstilstand og til at fejl i systemet selv bliver synlige. |


## Krav skema

Prioritet er kategoriseret fra 1 til 3, hvor 1 er vigtigst. "Node" refererer
til en registreret radiomodtager med tilhørende agent. "Operatør" refererer til
en bruger, der overvåger flåden. "Tekniker" refererer til den medarbejder, der
sendes ud til en station.

| ID | Krav | Krav opfyldt | Prioritet | Kategori | Beskrivelse |
|:----|:----------------|:------------|:-----------|:-------------|:--------------------------------------------|
| K1 | Åben protokol og selvregistrering | Opfyldt | 1 | Funktionalitet | Node og server kommunikerer gennem en åben, versioneret protokol beskrevet i et maskinlæsbart skema og offentliggjort uafhængigt af kildekoden. En node registrerer sig selv ved første opstart med identitet, placering, hardwarebeskrivelse og hvilke metrikker den kan levere. Serveren håndterer noder med forskellige måleevner, afviser ukendte protokolversioner entydigt og kan betjene flere versioner samtidigt i en overgangsperiode. |
| K2 | Egen signalbehandlingskæde | Delvist | 1 | Funktionalitet | Node-agenten udleder pr. kanal: signalstyrke, signal-støj-forhold, afvigelse mellem forventet og observeret bærebølgefrekvens, demodulationsfejlrate og spektrumsbelægning. Demodulationsfejlraten udledes for FM af RDS-datastrømmens blokfejlrate, så metrikken afspejler om signalet kan afkodes og ikke blot om der er energi til stede. Kæden, herunder frekvenstransformation, effektspektrumsestimering, demodulation og RDS-afkodning, implementeres i projektet og består ikke af opkald til et færdigt bibliotek. Fire af de fem metrikker udledes. Mangler RDS-forenden, der låser 57 kHz-underbærebølgen og producerer bitstrømmen til bloklaget, hvorfor demodulationsfejlraten ikke oplyses som evne. |
| K3 | Pålidelig indrapportering | Opfyldt | 1 | Funktionalitet | Agenten aggregerer målinger over et konfigurerbart tidsvindue og rapporterer statistiske sammendrag frem for hver enkelt rå måling. Ved netværksafbrydelse mellemlagres data lokalt og eftersendes ved genetablering, uden tab og uden dubletter. |
| K4 | Skalerbar tidsserielagring | Opfyldt | 1 | Funktionalitet | Serveren validerer indkomne målinger mod skemaet og mod fysisk meningsfulde værdiintervaller før persistering. Måledata lagres i en struktur designet til tidsserier og til forespørgsler over tidsintervaller, og lagringen vokser med flåden gennem en aldersbestemt opbevaringspolitik, hvor rå målinger fortættes til grovere opløsning med alderen. |
| K5 | Gennemskuelig afvigelsesdetektion | Opfyldt | 1 | Acceptance | Serveren vedligeholder en rullende baseline pr. node, kanal og metrik over et konfigurerbart vindue og detekterer afvigelser herfra. Detektionen bygger på robuste statistiske mål, der ikke forstyrres af enkeltstående udfald, beregnes løbende uden gennemløb af hele historikken, og enhver alarm kan forklares ud fra de data og den baseline, der udløste den. |
| K6 | Alarmens livscyklus og underretning | Opfyldt | 1 | Acceptance | En afvigelse rejser en alarm med tilstanden åben, kvitteret, under efterprøvning eller lukket. Alarmens historik bevares, herunder hvem der ændrede tilstanden hvornår og med hvilken begrundelse. Nye alarmer skubbes til åbne klienter i realtid og udsendes til abonnenter gennem mindst én asynkron kanal. En node der holder op med at rapportere behandles som en selvstændig hændelse. |
| K7 | Udkald og efterprøvning i felten | Opfyldt | 2 | Acceptance | En alarm kan omsættes til en arbejdsordre, der tildeles en navngiven tekniker og knyttes til den station, der skal besøges. Teknikeren registrerer på stedet, om fejlen fortsat er til stede, hvad årsagen vurderes at være, og hvilken handling der er foretaget. Resultatet tilbageføres til den udløsende alarm, så det kan opgøres, hvilke alarmer der var reelle. |
| K8 | Visualisering af måledata | Delvist | 1 | Acceptance | Webklienten viser et kort over flådens noder med aktuel status, tidsserier pr. kanal og metrik med valgbart tidsinterval, en spektrumvisning af det overvågede bånd, sammenligning af flere noder eller kanaler i samme visning, og en tidslinje der sammenholder måledata med alarmer og udførte udkald. Visningerne forbliver brugbare ved store tidsintervaller, hvilket forudsætter udlevering af fortættede data. Kort, tidsserier og fortættede intervaller er på plads. Spektrumvisningen, sammenligningen af flere noder og tidslinjen er ikke nået; båndet vises kun som båndudnyttelse over tid. |
| K9 | Sikkerhed | Delvist | 1 | Sikkerhed | Al kommunikation mellem node, server og klient er krypteret. Hver node autentificerer sig med egen legitimation og kan ikke indsende eller ændre data på vegne af en anden node. Brugeradgang styres gennem roller med adskilte rettigheder: administrator, operatør, tekniker og læser. En suspenderet node afvises ved indsendelse, og serveren begrænser indsendelsesfrekvens og payloadstørrelse pr. node. Kryptering, nodelegitimation, rollemodel og suspension er på plads. Mangler håndhævelse af indsendelsesfrekvens og payloadstørrelse, som protokollen fastsætter, men serveren endnu ikke afviser på. |
| K10 | Overvågning af systemets egen drift | Opfyldt | 2 | Drift | Systemet eksponerer nøgletal for sin egen tilstand: gennemløb i dataindtaget, kølængder, svartider, fejlrater og tidspunktet for detektionsmotorens seneste gennemløb. Udebliver dataindtag eller detektion ud over en fastsat grænse, rejser systemet selv en driftsalarm, så et stoppet system ikke fremstår som et system uden fejl. Node-agenten rapporterer desuden egen oppetid, belastning, temperatur og afvigelse på systemuret. |


# Afgrænsning

Systemet observerer og rapporterer. Det udfører ikke fejlretning, styrer ikke
sendere og foretager ingen automatiske indgreb i den overvågede infrastruktur.
Systemet modtager udelukkende og sender ikke på nogen frekvens.

Til demonstrationen anvendes en FM-sender som kontrolleret signalkilde, så en
forringelse kan fremkaldes gentageligt frem for at afvente, at en offentlig
sender forringes af sig selv. Senderen er prøveudstyr og indgår ikke i det
leverede system. Der anvendes en sender inden for den effektgrænse, der gælder
for tilladelsesfri anvendelse af FM-båndet, og demonstrationen kræver derfor
ingen sendetilladelse.

Det anvendte udstyr er eksempler og ikke krav. Systemet stiller krav til, at en
node kan levere et rå IQ-signal og tale protokollen, ikke til fabrikat eller
model. De valgte modtagere tilhører to forskellige familier med forskellige
måleevner, så den påstand bliver afprøvet.

Der bygges én fysisk node. Systemets opførsel ved større skala demonstreres med
simulerede noder, der kommunikerer gennem den samme protokol og det samme
dataindtag som den fysiske node. Serveren skelner ikke mellem de to.

Målingerne er relative til nodens egen historiske baseline. Modtagerne
kalibreres ikke mod et absolut referenceniveau, og systemet angiver derfor ikke
absolut feltstyrke. Systemet er ikke certificeret måleudstyr, og resultaterne kan
ikke anvendes som juridisk dokumentation.

Nodens geografiske placering angives af nodens ejer ved registrering. Der stilles
ikke krav om GPS-modtager på noden.

Afvigelsesdetektionen bygger på beskrevet statistik. Der anvendes ikke
maskinlæring, da enhver alarm skal kunne forklares ud fra de data, der udløste
den. Korrelation af afvigelser på tværs af noder, som ville kunne adskille en
senderfejl fra en lokal fejl, er ikke et krav i denne version.

Udkaldsdelen dækker tildeling, efterprøvning og afslutning af en arbejdsordre.
Den omfatter ikke ruteplanlægning, tidsregistrering, lagerstyring af reservedele
eller fakturering.

Systemet behandler ikke personoplysninger ud over nodeejerens og medarbejderens
kontooplysninger samt nodens angivne placering. Der optages, gemmes eller afkodes
ikke lydindhold fra de overvågede udsendelser. Kun metrikker om signalets
kvalitet lagres.

Der udvikles ikke en native mobilapplikation, og DAB+ understøttes ikke i denne
version.
