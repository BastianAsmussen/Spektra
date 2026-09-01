---
title: "Distribueret overvågning af radiosignalkvalitet"
subtitle: "Procesrapport"
author:
  - name: "Bastian Almar Wolsgaard Asmussen"
    affiliation: '`#text(size: 0.8em)[Vejledere: Simon Hoxer Bønding og Lars Thise Pedersen]`{=typst}'
date: "24. september 2026"
lang: da-DK
---

# Læsevejledning

Rapporten beskriver forløbet og de valg, der formede Spektra. Produktrapporten
beskriver selve produktet, dets krav og den tekniske dokumentation.

Produktrapporten bør læses først, fordi den viser, hvad systemet er. Rapporten
viser, hvordan det blev til.

Systemets overordnede arkitektur er vist i produktrapportens bilag 1.
Diagrammet viser systemets fire komponenter og de to grænseflader mellem dem,
som teknologivalgene i kapitlet Metode- og teknologivalg bygger på.

Databasens opbygning er vist i produktrapportens bilag 2.

Systemet er publiceret på <https://spektra.asmussen.tech> og kræver ingen
installation for at blive afprøvet. Adressen, administratorens brugernavn og
adgangskode samt en vejledning til brugen af systemet findes i produktrapportens
kapitel Brugervejledning.

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


# Projektplanlægning

Projektperioden løber fra mandag den 31. august 2026 til torsdag den 24.
september 2026, hvor begge rapporter afleveres. Perioden rummer 19 arbejdsdage.
Fremlæggelsen ligger den 1. oktober 2026, og ugen mellem
aflevering og fremlæggelse er afsat til demonstrationen og forberedelsen af
fremlæggelsen.

Planlægningen blev lavet, før udviklingen begyndte. Det væsentligste udbytte
af arbejdet var ikke selve datoerne, men at projektets omfang blev gennemtænkt
komponent for komponent, før den første linje kode blev skrevet. Et estimat
tvinger en til at tage stilling til, hvad en opgave egentlig indeholder, og
flere af teknologivalgene blev truffet på baggrund af den øvelse.


## Estimeringsmetode

Hver opgave har ét estimat i hele timer, sat ud fra opgavens indhold.
Arbejdsfordelingens andele er udledt direkte af de samme timer. Estimaterne er
mest usikre for signalbehandlingskæden, som der var mindst erfaring med på
forhånd, og som står for 49 af de 235 timer.

Tidsplanen ligger som en tekstfil i repositoryet og tegnes derfra, så figur og
plan ikke kan komme ud af trit.

- Hver linje er en opgave.
- Et krav får `K` og kravets nummer.
- En forudsætning får `P`, en løbende aktivitet `L`.
- Timerne står i tabellen for hver fase.

At en opgave strækker sig over flere dage betyder ikke, at der arbejdes på den
hele tiden. Opgaven kan vente på noget andet eller være bundet til dele af
døgnet, som måleopgaver, der kræver at noden har kørt et stykke tid først.

Weekender tæller med i diagrammet, fordi det samlede estimat forudsætter arbejde
uden for skematiden.


## Estimeret tidsplan

Den estimerede tidsplan er vist i bilag 1.


### Fase 0: forberedelse og planlægning, 31/08 til 01/09

| ID | Opgave | Timer |
| --- | --- | ---: |
| P1 | Case beskrivelse, problemformulering og afgrænsning | 3 |
| P2 | Kravspecifikation K1 til K10 med kategorier og prioritet | 4 |
| P3 | Undersøgelse af SDR-hardware og indkøbsanmodning til vejleder | 2 |
| P4 | Teknologivalg, opsætning af repository og udviklingsmiljø | 4 |
| | **I alt** | **13** |


### Fase 1: protokol, dataindtag og publicering, 02/09 til 07/09

| ID | Opgave | Timer |
| --- | --- | ---: |
| K1.1 | Protokolskema version 1 for node, måling, dataindtag og helbred | 5 |
| K1.2 | Tjeneste til dataindtag og nodens selvregistrering | 10 |
| P5 | Databaseskema og migrationer for brugere, roller, noder og kanaler | 8 |
| K4.1 | Validering mod skema og mod fysisk meningsfulde værdiintervaller | 3 |
| P6 | Emulator til simuleret nodeflåde | 4 |
| P7 | Byggeautomatik, testdatabase og betinget udrulning | 4 |
| P8 | Idriftsættelse af server med reverse proxy, TLS og navneopslag | 8 |
| | **I alt** | **42** |


### Fase 2: signalbehandling og tidsserielagring, 08/09 til 14/09

| ID | Opgave | Timer |
| --- | --- | ---: |
| K2.1 | Vinduesfunktion og effektspektrumsestimering | 16 |
| K2.2 | Filtrering, decimering og FM-demodulation | 12 |
| K2.3 | RDS-afkodning og udledning af blokfejlrate | 16 |
| K2.4 | Udledning af de fem metrikker pr. kanal | 5 |
| K3.1 | Aggregering over konfigurerbart tidsvindue | 3 |
| K3.2 | Lokal mellemlagring og eftersendelse uden tab og dubletter | 8 |
| K4.2 | Partitionerede tidsserietabeller og vedligeholdelsesjob | 6 |
| K4.3 | Fortætningsjob og aldersbestemt opbevaringspolitik | 7 |
| P9 | Fysisk opbygning af node med modtager, antenne og systemimage | 6 |
| | **I alt** | **79** |


### Fase 3: detektion, alarmering og webklient, 15/09 til 21/09

| ID | Opgave | Timer |
| --- | --- | ---: |
| K5 | Rullende baseline og forklarbar afvigelsesdetektion | 12 |
| K6.1 | Alarmens livscyklus og bevaret historik | 5 |
| K6.2 | Realtidsudsendelse af nye alarmer til åbne klienter | 3 |
| K6.3 | Asynkron underretning til abonnenter | 2 |
| K6.4 | Detektion af nodetavshed som selvstændig hændelse | 2 |
| K8.1 | Kort over flåden med aktuel status pr. node | 4 |
| K8.2 | Tidsserier pr. kanal og metrik med valgbart interval | 5 |
| K8.3 | Spektrumvisning af det overvågede bånd | 4 |
| K8.4 | Sammenligning af flere noder samt tidslinje med alarmer og udkald | 5 |
| K9.1 | Roller og adskilte rettigheder for de fire brugertyper | 6 |
| K10.1 | Nøgletal for systemets egen driftstilstand | 3 |
| K10.2 | Driftsalarm ved udeblevet dataindtag eller detektion | 2 |
| K10.3 | Nodens rapportering af oppetid, belastning, temperatur og urafvigelse | 2 |
| | **I alt** | **55** |


### Fase 4: udkald, hærdning og aflevering, 22/09 til 24/09

| ID | Opgave | Timer |
| --- | --- | ---: |
| K7 | Arbejdsordrer, tildeling og efterprøvning i felten | 6 |
| K9.2 | Afvisning af suspenderede noder samt frekvens- og payloadgrænser | 3 |
| P10 | Udførelse og dokumentation af testkonditioner | 5 |
| P11 | Færdiggørelse og korrektur af begge rapporter | 8 |
| | **I alt** | **22** |


### Løbende aktiviteter, 31/08 til 24/09

| ID | Opgave | Timer |
| --- | --- | ---: |
| L1 | Logbog med én række pr. projektdag | 3 |
| L2 | Rapportskrivning parallelt med udviklingen fra uge 2 | 16 |
| | **I alt** | **19** |


### Fase 5: fremlæggelse, 25/09 til 29/09

| ID | Opgave | Timer |
| --- | --- | ---: |
| P12 | Demonstration med dæmpeled og fremlæggelsesplan | 5 |
| | **I alt** | **5** |


### Samlet estimat

| Fase | Estimat i timer |
| --- | ---: |
| Fase 0: forberedelse og planlægning | 13 |
| Fase 1: protokol, dataindtag og publicering | 42 |
| Fase 2: signalbehandling og tidsserielagring | 79 |
| Fase 3: detektion, alarmering og webklient | 55 |
| Fase 4: udkald, hærdning og aflevering | 22 |
| Løbende aktiviteter | 19 |
| Fase 5: fremlæggelse | 5 |
| **I alt** | **235** |

Estimatet lander på 235 timer mod omkring 140 timers skematid over de 19
arbejdsdage. Forskellen hentes i weekender og på længere dage.

K7 og K10 har prioritet 2 og er de første, der skæres i, hvis tiden ikke
rækker. Rapportskrivningen ligger som et løbende spor fra uge 2, så den ikke
konkurrerer med de sidste krav.

Alternativet var at skære kravene ned, til de passede til 140 timer. Det havde
givet en plan, der var nemmere at holde, og et produkt, der ikke svarede på
problemformuleringen. Måledata uden detektion, eller detektion uden fysisk
efterprøvning, løser det halve problem.


## Arbejdsfordeling

Projektet er udført af én person, så arbejdsfordelingen fordeler tid mellem
arbejdstyper.

| Arbejdstype | Estimat i timer | Andel |
| --- | ---: | ---: |
| Udvikling og implementering | 171 | 73 % |
| Dokumentation og rapportskrivning | 34 | 15 % |
| Infrastruktur og drift | 12 | 5 % |
| Hardware | 8 | 3 % |
| Test | 5 | 2 % |
| Fremlæggelse | 5 | 2 % |
| **I alt** | **235** | **100 %** |

Fordelingen er tilrettelagt, så en blokeret opgave ikke standser projektet. Når
en udviklingsopgave afventer noget udefra, eksempelvis levering af hardware eller
svar fra vejleder, flyttes indsatsen til dokumentation eller til den næste
uafhængige opgave i planen. Rapportsporet og logbogen er derfor løbende
aktiviteter og ikke faser.


# Metode- og teknologivalg

Hardwaren binder resten af stakken. Modtagerens opløsning afgør, hvilke
metrikker der overhovedet kan udledes, og nodens regnekraft afgør, hvor dyr
signalkæden må være pr. sekund signal. Begge dele er fastlagt, før der er skrevet
en linje kode.


## Hardware

Problemformuleringen efterspørger en løsning til *eksisterende* radiomodtagere.
Det er en påstand om, at systemet ikke er bundet til bestemt udstyr, og en
påstand af den slags bliver først troværdig, når den er afprøvet. Udstyret er
derfor valgt som eksempler og ikke som krav. Enhver modtager, der kan levere et
rå IQ-signal, kan indgå i flåden.

Der er valgt modtagere fra to forskellige familier frem for flere eksemplarer af
den samme, fordi K1 kræver, at serveren håndterer noder med forskellige
måleevner. Med kun én modtagertype ville det krav aldrig blive afprøvet, og protokollens beskrivelse af nodens evner ville
være ubrugt kode. Noden oplyser ved registrering, hvilke metrikker den kan
levere, og en modtager, der ikke kan levere en metrik, udelader den i stedet for
at rapportere et gæt.


### Modtagere

Som primær modtager anvendes RTL-SDR Blog V4. Den afgørende egenskab er ikke
prisen, men oscillatoren. En generisk DVB-T-dongle leveres med et krystal med en
tolerance på ~30 PPM. Ved 100 MHz svarer det til en afvigelse på op mod 3 kHz,
og afvigelsen vandrer, mens enheden varmer op over det første kvarters tid. K2 kræver, at noden måler afvigelsen mellem forventet og observeret
bærebølgefrekvens. Med et generisk krystal ville den metrik måle modtagerens egen
drift frem for senderens tilstand. En temperaturkompenseret oscillator med 1 PPM
svarer til ~100 Hz ved samme frekvens, altså en faktor 30, og først der
beskriver metrikken det, den påstår at beskrive.

Som anden modtagerfamilie anvendes Airspy Mini. Den har 12 bits opløsning mod
DVB-T-donglens 8 og kan levere en større båndbredde.

SDRplay blev fravalgt. Enheden er teknisk udmærket, men dens driver udleveres som
en lukket binær komponent, og en node, hvis signalvej hviler på en komponent, der
ikke kan læses, passer dårligt til et projekt, hvis egen signalbehandlingskæde er
et krav.


### Nodens computer

Noden bygges på en Raspberry Pi 5 med 4 GB hukommelse. Kravet til maskinen følger
af signalbehandlingskæden. Filtrering og decimering står for omkring 60% af
kædens regnetid, effektspektrumsestimeringen for knap en tredjedel og resten er
demodulation og metrikudledning. Hele kæden bruger under en femtedel af én kerne
på en almindelig arbejdsstation og skønsmæssigt to tredjedele af én kerne på
nodens maskine, fordelt over fire kanaler. Der er hovedrum, men ikke rigeligt,
og det er med vilje: en maskine med rigeligt ville ikke vise, at flåden kan
bygges billigt.

Noden var oprindeligt planlagt med to lagermedier. Den starter gennem u-boot
(`hardware.raspberry-pi.firmware.uboot.enable` i `nix/node/host.nix`), og u-boot
mangler drivere til USB og PCIe på Raspberry Pi 5. Opstartsfilerne skal
ligge på SD-kortet, og et SD-kort er det forkerte medie at skrive løbende til.
Roden skulle have ligget på en USB-tilsluttet SSD, og SD-kortet skulle have holdt
firmware og Linux-kerner alene.

Den opdeling blev droppet under indkøbet. Den SSD, der lå inden for budgettet,
kom fra en leverandør, som ved kontrol viste sig ikke at være reel, og hos de
reelle leverandører lå prisen over, hvad anskaffelsen kunne bære. Noden kører
derfor udelukkende fra SD-kortet, som `nix/node/host.nix` beskriver.

Konsekvensen er afgrænset, fordi agenten skriver lidt. Rå samples rammer aldrig
disken; de forbruges i hukommelsen, efterhånden som driveren leverer dem. Agenten
persisterer kun nodens identitet, dens legitimation, den cachede kanalplan og
offlinejournalen ved netværksudfald, og systemet skriver sine logfiler. Et
SD-kort, der slides op på den belastning, er en billig og udskiftelig del.

En lille x86-maskine ville have givet mere regnekraft for pengene. Den blev
fravalgt, fordi projektets påstand er, at et måleapparat af denne type kan bygges
af billige dele og udbredes i antal. Den påstand afprøves kun, hvis noden faktisk
bygges af billige dele.


### Signalkilde til demonstrationen

Til demonstrationen anskaffes en FM-sender af den type, der sælges til brug
sammen med en telefon eller en bilradio.

En demonstration af afvigelsesdetektion kræver en forringelse, der kan styres og
gentages, og det kan en rigtig radioudsendelse ikke give. En offentlig sender kan
ikke forskydes i frekvens, dens støjgulv kan ikke hæves på kommando, og dens egen
variation over tid indgår i målingen som støj, der ikke kan skilles fra den
forringelse, der skal vises.

Med en egen sender kan de fire metrikker, noden udleder, påvirkes hver for sig.
En forskudt frekvens flytter bærebølgeafvigelsen. Et dæmpeled mellem sender og
modtager giver en kendt dæmpning, der flytter signalstyrke og
signal-støj-forhold. En slukket sender tømmer kanalen og flytter
spektrumsbelægningen. Mod en offentlig sender kan kun dæmpeleddet bruges.
Demodulationsfejlraten kunne forringes gennem senderens datastrøm, men noden
udleder den først, når RDS-forenden er på plads (K2).

Senderen er prøveudstyr og ikke en del af det leverede system. Systemet selv
modtager udelukkende. Senderen holder sig inden for effektgrænsen for
tilladelsesfri brug af FM-båndet; en kraftigere sender ville kræve en
sendetilladelse.

Detektoren bygger sin baseline over et rullende vindue på 28 døgn, og en sender,
der tændes samme dag, som forringelsen skal vises, har ingen baseline at afvige
fra. Enten kører senderen gennem hele vinduet i forvejen, eller også konfigureres
den node med et kortere vindue.
