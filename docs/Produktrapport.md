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


# Teknisk produktdokumentation


## Overordnet arkitektur

Systemet består af fire komponenter og to grænseflader. Arkitekturen er vist i
bilag 1.

**Node-agenten** afvikles på en enkeltkortscomputer med en tilsluttet
SDR-modtager. Den henter sin kanalplan fra serveren, sampler hver tildelt kanal
efter tur i et kanalophold af fast længde (`--dwell-seconds`), udleder fem
kvalitetsmetrikker af det rå IQ-signal,
aggregerer dem over et konfigurerbart tidsvindue og afleverer ét statistisk
sammendrag pr. kanal og metrik. Rå samples forlader aldrig noden.

**Den centrale server** modtager, validerer og persisterer måledata,
vedligeholder en rullende baseline pr. node, kanal og metrik, detekterer
afvigelser, administrerer alarmer og arbejdsordrer, udsender notifikationer og
overvåger sin egen driftstilstand.

**Webklienten** er den operative grænseflade. Serveren renderer HTML-fragmenter,
som klienten indsætter uden at genindlæse siden, og nye alarmer og
nodetilstande skubbes over en WebSocket-forbindelse.

**Protokolspecifikationen** er en selvstændig, versioneret leverance. Den
beskriver dataformat, transport, autentificering og versioneringsregler i et
maskinlæsbart skema, så en tredjepart kan implementere en node uden adgang til
node-agentens kildekode.

Mellem node og server tales gRPC over protobuf, hvor skemaet er kontrakten og en
fremmed implementation kan generere sin egen klient af den. Mellem server og
webklient tales HTTP med HTML-svar samt en WebSocket til hændelser. En node og
en browser har ikke samme behov, og et fælles format til begge ville tvinge det
ene til at bære det andets begrænsninger.


### Dataflow

1. Node-agenten sampler kanalen og udleder metrikkerne lokalt.
2. Agenten aggregerer over rapportvinduet og sender ét `MeasurementReport`.
3. Serveren validerer rapporten mod skemaet og mod fysisk meningsfulde
   værdiintervaller.
4. Målingerne persisteres i den partitionerede `measurements`-tabel.
5. Fortætningsjobbet opsummerer de rå vinduer i time-, dags- og ugeopløsning.
6. Detektoren bygger en baseline pr. time på døgnet og sammenholder de seneste
   vinduer med den.
7. En afvigelse rejser en alarm, som skubbes til åbne klienter og kan sendes
   videre til abonnenter over ntfy.
8. En operatør omsætter alarmen til en arbejdsordre, en tekniker efterprøver
   fejlen på stedet, og resultatet føres tilbage til den udløsende alarm.

Trin 5 og 6 kører som baggrundsjob på hver sin timer og ikke som en del af
dataindtaget. En langsom fortætning må ikke kunne forsinke modtagelsen af en
måling, og en stoppet detektor må ikke kunne fremstå som en flåde uden fejl.
Begge tilstande er selvstændigt overvåget.

Hele systemet bygges, testes og udrulles automatisk fra det samme repository.
Et push kører kompilering, statisk analyse og den fulde testsuite, og
udrulningen til den publicerede server sker først, når testkørslen er
gennemført uden fejl. Værtens konfiguration er deklareret i samme repository
som applikationen, så en udrulning ikke kan komme til at afvige fra det, der
blev testet. Se kapitlet Drift og udrulning.


## Protokol

Protokollen er en selvstændig leverance og ikke en intern detalje i serveren.
Den ligger i sin egen pakke, `protocol/`, og består af syv skemafiler under
`proto/v1/`. En tredjepart kan generere en klient direkte af skemaet og
implementere en node uden adgang til node-agentens kildekode, som K1 kræver.

| Skemafil | Indhold |
| --- | --- |
| `common.proto` | Modulationer, metrikker, statistisk sammendrag og rapportplan |
| `node.proto` | Selvregistrering: identitet, placering, hardware og evner |
| `channel.proto` | Kanalplanen, som serveren tildeler den enkelte node |
| `measurement.proto` | Målerapporten og kvitteringen for den |
| `health.proto` | Nodens rapportering af sin egen driftstilstand |
| `live.proto` | Live-inspektion af et enkelt kanalophold, som aldrig persisteres |
| `ingest.proto` | Tjenesten `NodeIngest` med de seks kald |


### Versionering

Versionen optræder to steder. Den er en del af pakkenavnet, `spektra.v1`, så
hver version er sin egen tjeneste med sine egne stier, og den gentages som et
eksplicit felt `protocol_version` på hver eneste anmodning.

En anmodning til en sti, serveren ikke længere registrerer, besvares med
`UNIMPLEMENTED`. En anmodning, hvis `protocol_version` ikke svarer til den
tjeneste, den rammer, afvises med `INVALID_ARGUMENT`. Så længe serveren
registrerer flere versioner samtidigt, betjenes de side om side; det er den
overgangsperiode, K1 kræver. Feltet er ikke redundant i forhold til
pakkenavnet: det fanger en klient, der er genereret af ét skema og peget mod en
anden tjeneste. Uden feltet ville den fejl først vise sig som mærkelige data.


### Selvregistrering og evner

En node registrerer sig selv ved første opstart med en identitet, den selv
persisterer på tværs af genstarter, et visningsnavn, en placering, en
beskrivelse af modtagerkæden og en liste over de metrikker og modulationer, den
kan levere. Serveren svarer med nodens id, en bearer-legitimation til alle
senere kald og sit eget ur.

Registrering med en identitet, serveren allerede kender, afvises med
`ALREADY_EXISTS`. En node med en gemt legitimation bruger den og registrerer sig
ikke igen.

`Capabilities` er grunden til, at serveren kan håndtere noder med forskellige
måleevner. En modtager, der ikke kan udlede en metrik, udelader den i stedet for
at rapportere et gæt, og serveren behandler fravær som fravær og ikke som nul.


### Kanalplanen

En node bestemmer ikke selv, hvad den lytter på. Serveren ejer kanalplanen, og
noden beder om sin egen, så en modtager kan omdisponeres uden at røre ved noden
eller udrulle den igen.

Planen bærer en monotont voksende version. Noden spørger med den version, den
har, og genopbygger kun sin sampleplan, når serveren svarer med en højere. En
uændret plan koster én lille rundtur. Versionen vedligeholdes ikke af
applikationskoden, men af en databasetrigger på `node_channels`, så den ikke kan
komme til at stå stille, fordi en skrivning gik uden om den rigtige funktion.


### Rapportkadencen

Hver kvittering kan bære en `ReportSchedule` med det tidspunkt, serveren ønsker
den næste levering. Noden måler på sin egen kadence, og planen flytter kun det
øjeblik, den taler, så de aggregeringsvinduer, den producerer, forbliver
sammenhængende.

Formålet er at sprede en flåde, der ellers ville rapportere i takt. Hundrede
noder, der startes samtidigt, forbliver i fase for altid og forvandler en jævn
belastning til en spids én gang i minuttet. Tidspunktet er på serverens ur, og
hver besked, der bærer en plan, bærer også serverens tid, så noden regner i
differencer og urforskellen mellem de to ophæver sig selv.

Noden begrænser det tidspunkt, den får: aldrig tidligere end nu, og aldrig længere
ude end dens egen konfigurerede grænse. En forkert eller fjendtlig server kan
ikke bringe en flåde til tavshed.


### Transport og grænser

Tjenesten tales over gRPC. Alle kald undtagen `RegisterNode` autentificeres med
nodens egen bearer-legitimation, som sendes i `authorization`-metadata på hvert
kald. Transporten er TLS i drift, termineret foran tjenesten, så protokollen
ikke selv forhandler kryptering.

Protokollen fastsætter grænser for indsendelsesfrekvens og payloadstørrelse pr.
node. En rapport over grænsen afvises med `RESOURCE_EXHAUSTED` og skal forsøges
igen senere, aldrig hurtigere. Serveren håndhæver endnu ikke grænserne, jf. K9.

En målerapport er atomisk. Fejler en enkelt kanal eller metrik valideringen,
afvises hele rapporten med `INVALID_ARGUMENT`. Alternativet, delvis accept,
ville efterlade noden i tvivl om, hvad der nåede frem, og eftersendelsen efter
et netværksudfald må ikke bygge på den tvivl.
