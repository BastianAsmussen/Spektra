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


## Node-agent

Agenten styrer en SDR-modtager og består af en signalkæde og en gRPC-klient.
Den stiller ind på de kanaler, serveren har tildelt den, udleder kvalitetsmetrikker af rå IQ,
aggregerer over et vindue og rapporterer sammendraget. Rå samples forlader
aldrig noden.


### Identitet og legitimation

En node registrerer sig én gang. Den identitet, den registrerede sig under, det
id serveren tildelte, og den legitimation den fik udstedt, skrives til nodens
tilstandsmappe og genindlæses ved opstart. Uden det ville hver genstart fremstå
som en ny node, og den rullende baseline, detektoren afhænger af, ville starte
forfra hver gang.


### Den cachede kanalplan

Da noden ikke selv bestemmer, hvad den lytter på, har en node, der aldrig har
talt med serveren, heller ikke noget at måle. Det er korrekt ved første opstart
og forkert alle andre gange: målingerne betyder mest, mens en stations
netforbindelse er nede, og en genstart i det vindue ville ellers efterlade
modtageren tomgangskørende, indtil nettet kom tilbage.

Planen skrives ved siden af identiteten og genindlæses ved opstart. Den
bærer sin egen version, så den første vellykkede forespørgsel efter en genstart
enten bekræfter den eller erstatter den.


### Måleløkken

Læsninger fra `SoapySDR` blokerer, og løkken kører derfor på sin egen
operativsystemtråd frem for på den asynkrone køretid. En blokerende læsning
inde i en asynkron opgave ville standse alle andre opgaver på den samme
arbejdertråd, herunder den, der afleverer rapporter. Tråden ejer modtageren og
signalkæden og afleverer færdige målinger til den asynkrone side over en kanal.

Adgangen til modtageren er abstraheret ét sted. `SoapySDR` opregner både RTL-SDR
og Airspy gennem den samme grænseflade, så det eneste, der adskiller de to
familier, er drivernøglen og den samplerate, hardwaren accepterer. Læsninger
fylder en buffer, kalderen ejer: ved 2,4 MSPS læser agenten i størrelsesordenen
hundrede blokke i sekundet, og en ny allokering pr. blok ville placere
allokatoren midt i samplestien.

Der findes desuden en syntetisk signalkilde, som producerer IQ uden hardware.
Med den kører alle agentens enhedstests uden en modtager, også i
byggeautomatikken.


### Signalbehandlingskæden

Hele kæden er skrevet i projektet, og det gælder også frekvenstransformationen.
Den er en radix-2 Cooley-Tukey-FFT, som er udviklet som en del af dette arbejde
og udgivet selvstændigt som `spektra-fft`. Den egne pakke gør den ikke til
tredjepartskode: den er skilt ud, fordi en transformation er brugbar langt ud
over dette system og kan vedligeholdes og versioneres for sig. K2 kræver, at
kæden ikke består af opkald til et færdigt bibliotek.

**Vindue.** Et endeligt udsnit af et kontinuert signal er i sig selv et
rektangulært vindue over det, og et rektangulært vindues egen transformation
har første sidelap kun 13 dB nede, så en stærk bærebølge smøres ud over de
bins, en svag måles i. Derfor bruges et Hann-vindue, som sænker første sidelap
til 31 dB mod en bredere hovedlap.

**Effektspektrum.** Spektret estimeres med Welchs metode, altså som et
gennemsnit af mange korte spektre. Metoden bytter frekvensopløsning for et
stabilt støjgulv, og det er støjgulvet, fire af de fem metrikker aflæses mod.
Ved 2,4 MSPS leverer modtageren 2,4 millioner komplekse samples i sekundet, og
de holdes ikke i hukommelsen: segmenter forbruges, efterhånden som driveren
leverer dem, og kun den løbende effektsum pr. bin overlever. Hukommelsesforbruget
følger transformationens længde, uanset hvor længe kanalopholdet varer.

**Filtrering og decimering.** Spektrumsstien måler hele spændet på 2,4 MHz,
men demodulationen skal kun bruge den ene kanal i midten af det. Et FIR-filter
ned til kanalen efterfulgt af at kassere ni ud af ti samples koster én
foldning og køber en tifoldig reduktion i alt, hvad der ligger efter.

**FM-demodulation.** Informationen i en FM-bærebølge ligger i, hvor hurtigt
dens fase drejer. Demodulatoren er én kompleks multiplikation og én
`atan2` pr. sample: argumentet af `z[n] * konj(z[n-1])` er den fase, der er
tilbagelagt på én sampleperiode, og divideret med den periode giver det
frekvens.

**Metrikker.** Signalstyrke, signal-støj-forhold, bærebølgeafvigelse og
spektrumsbelægning udledes alle af det midlede effektspektrum.

Hele kæden regner i `f32`. Transformationen har dobbelt gennemløb i `f32` i
forhold til `f64` under NEON på nodens processor, og en 8 bits ADC, der føder
en transformation på 32.768 punkter, ligger ikke i nærheden af præcisionsgulvet.
Aggregeringen udvides til `f64`, fordi feltet i protokollen er `double`.


### RDS og demodulationsfejlraten

Demodulationsfejlraten er i protokollen defineret som andelen af modtagne
RDS-blokke, der ikke består deres CRC. Metrikken er derfor en RDS-afkodning,
uanset om resten af RDS er interessant.

Bloklaget er implementeret: differentiel afkodning, CRC, de fem offsetord,
gruppesynkronisering og selve fejlraten. Den analoge forende, der producerer
bitstrømmen, er ikke. Den kræver, at 19 kHz-piloten genfindes i multipleksen og
tredobles for at låse den undertrykte 57 kHz-underbærebølge, et tilpasset
filter til bifasesignalet og en timingsløkke ved 1187,5 baud.

Indtil den del er på plads, kalder den kørende agent ikke ind i bloklaget, og
metrikken optræder ikke blandt de evner, noden oplyser ved registrering.
Protokollen er evnebaseret, og en node, der udelader en metrik, den ikke kan
udlede, opfører sig korrekt. En node, der rapporterer et tal, den ikke har
målt, gør ikke.


### Aggregering og offlinejournal

Et kanalophold giver én værdi pr. metrik pr. kanal. Aggregeringsvinduet samler
dem til et statistisk sammendrag, og kun sammendraget krydser netværket.

Forbindelsen til serveren oprettes først, når der er noget at sende. En agent,
hvis server er uopnåelig ved opstart, skal stadig komme op, blive ved med at
sample og mellemlagre. Færdige vinduer skrives til en lokal journal og
eftersendes, når forbindelsen er tilbage.

Dubletter er ikke agentens problem alene: serverens unikke nøgle på node,
kanal, metrik og vinduesstart gør en gentaget levering virkningsløs. Det er den
anden halvdel af garantien i K3.


### Nodens eget helbred

En node, der er ved at blive dårligere, skal være synlig som netop det og ikke
som en station, der pludselig blev tavs. Agenten læser oppetid, belastning og
processortemperatur fra procfs og sysfs og beregner sin urafvigelse af de
servertidsstempler, registreringen og hver kvittering bærer. Alle aflæsninger
er valgfrie: en manglende fil på en udviklingsmaskine rapporterer nul frem for
at få kørslen til at fejle.


### Live-inspektion

En operatør med et panel åbent vil se noden nu og ikke om op til to minutter,
når vinduet er lukket og leveret. En live-prøve er ét kanalophold, sendt som det
måles, og serveren gemmer intet af den.

Det koster ingen ekstra signalbehandling, da det kanalophold, der i forvejen
føder aggregeringen, blot sendes med her. En session ændrer, hvad der forlader
noden, og intet ved, hvad den måler. Noden ringer selv ud og holder
forbindelsen åben, så serveren aldrig skal kunne nå noden udefra, og et klik
fra en operatør lander inden for et sekund.


## Backend


### Database

Databasen er PostgreSQL. Skemaet er vist i sin helhed i bilag 2 og opbygges af
15 migrationer, der versioneres sammen med kildekoden og køres af serveren selv
ved opstart. Tabellerne falder i fire grupper: adgang, flåde, måledata og
hændelser.

| Gruppe | Tabeller |
| --- | --- |
| Adgang | `roles`, `users`, `sessions` |
| Flåde | `nodes`, `node_credentials`, `channels`, `node_channels` |
| Måledata | `measurements`, `rollups`, `node_health` |
| Hændelser | `alarms`, `alarm_events`, `work_orders` |


#### Normalisering og relationer

Skemaet er på tredje normalform. Hver kendsgerning står ét sted, og
afhængigheder går udelukkende gennem primærnøgler. Rollenavne og
rollebeskrivelser ligger i `roles` frem for som en tekstkolonne på `users`, og
en kanals frekvens og modulation ligger i `channels` frem for at blive gentaget
i hver måling.

`nodes.hardware` og `nodes.capabilities` er af typen `JSONB` og ikke
normaliserede ud i egne tabeller. Felterne beskriver en modtagerkæde, hvis
sammensætning protokollen har lov til at udvide, og en tabel pr. felt ville
kræve en migration, hver gang en ny modtagertype kunne oplyse noget mere om sig
selv. `JSONB` lagres binært og kan indekseres, i modsætning til tekstlagret
JSON, hvor hver forespørgsel skal gennemlæse strengen igen.

`alarms.explanation` er ligeledes `JSONB`. Den bærer de tal, der udløste netop
den alarm: baselinens centrum, dens spredning, den observerede værdi og antallet
af vinduer uden for båndet. K5 kræver, at enhver alarm kan forklares ud fra det
datagrundlag, der rejste den, og en alarm, der først kan forklares ved at
genberegne baselinen bagefter, opfylder ikke kravet.

Fem opregnede typer er lagt i databasen som rigtige `ENUM`-typer frem for som
tekstkolonner: `modulation`, `metric`, `alarm_state`, `work_order_status` og
`rollup_resolution`. Databasen afviser en ugyldig tilstand, uanset hvilken
vej den kommer ind, og typerne genfindes direkte i Rust-modellerne.


#### Sletning og bevarelse

Fremmednøglerne bærer to forskellige politikker.

Data, der kun giver mening sammen med sin node, ryddes med noden:
`ON DELETE CASCADE` på målinger, fortættede målinger, legitimation,
helbredsrapporter og alarmer. Data, der dokumenterer, hvad et menneske gjorde,
bevares: `alarm_events.changed_by_user_id` og `nodes.owner_id` er
`ON DELETE SET NULL`, og en bruger deaktiveres med et flag i stedet for at blive
slettet, fordi alarmhændelser og arbejdsordrer navngiver vedkommende. En
slettet bruger ville tømme historikken for, hvem der kvitterede for hvad.


#### Tidsserielagring

`measurements` er den eneste tabel, der vokser med flåden gange tiden, og den er
den eneste, der er partitioneret. Tabellen er erklæret
`PARTITION BY RANGE (window_start)` med én partition pr. døgn og en
`measurements_default` som opsamling.

Forespørgsler over et tidsinterval kan udelade de partitioner, der ligger uden
for intervallet, og opbevaringspolitikken kan slette en dags data ved at droppe
en tabel i stedet for at slette rækker. En `DELETE` efterlader døde rækker, som
`VACUUM` skal rydde op i, mens et `DROP TABLE` frigiver pladsen med det samme.

Fortættede målinger ligger i `rollups` med tre opløsninger, time, dag og uge,
og er ikke partitioneret. De er per definition langt færre end de rå rækker, og
en partitionering af dem ville koste vedligeholdelse uden at give noget igen.


#### Indeksering

Hvert indeks i skemaet svarer til en konkret forespørgsel.

`idx_measurements_lookup` er `UNIQUE` på `(node_id, channel_id, metric,
window_start)`. Det er ikke et ydelsesindeks, men et korrekthedsindeks: en node,
der har mellemlagret data under et netværksudfald og eftersender dem, kan
komme til at sende et vindue, den allerede har leveret. Nøglen gør dataindtaget
idempotent, og det er den halvdel af K3, der ellers ville være umulig at
garantere.

`idx_measurements_recent` på `(window_start)` findes, fordi detektoren
gennemløber den seneste time på tværs af hele flåden frem for én serie ad
gangen. `idx_measurements_lookup` leder med `node_id` og kan ikke betjene den
forespørgsel. Partitionsudeladelsen indsnævrer til et døgn, og dette indeks
indsnævrer til timen.

`idx_rollups_bucket` på `(resolution, bucket_start)` findes af samme grund:
detektoren bygger hele flådens baseline i ét gennemløb og beder om et fast sæt
starttidspunkter for alle noder, og det kan `idx_rollups_lookup` med `node_id`
forrest ikke betjene.

`idx_alarms_open` er et partielt indeks på `(node_id, channel_id, metric)` med
betingelsen `WHERE state <> 'closed'`. Hver eneste læsning af tabellen beder om
de alarmer, der ikke er lukkede, og et btree-indeks har ingen strategi for
`<>`, så et almindeligt indeks på kolonnen aldrig ville blive valgt. Det
partielle indeks indeholder desuden kun de rækker, forespørgslen kan returnere,
så det forbliver lille, uanset hvor mange lukkede alarmer der er akkumuleret.


### API

Serveren eksponerer gRPC til noderne og HTTP til webklienten.


#### Dataindtag over gRPC

Tjenesten `NodeIngest` bærer de seks kald, protokollen definerer. Valideringen
er delt i to trin. `validate.rs` kontrollerer det, der kan afgøres ud fra
beskeden alene: at protokolversionen passer, at vinduet
slutter efter det begynder, at en kanal kun optræder én gang, og at hver metrik
ligger inden for et fysisk meningsfuldt interval. Intervallerne er defineret i
protokolpakken og ikke i serveren, så en tredjeparts node kan læse de samme
grænser af skemaet.

`persist.rs` udfører derefter skrivningerne. En rapport skrives i én
transaktion, og dublet-nøglen på `measurements` gør, at en gentagen levering
hverken skriver noget eller fejler.

Nodens sidst sete tidspunkt opdateres betinget, og betingelsen er ikke en
optimering af skrivningen, men af låsen. En ubetinget opdatering tager en
rækkelås på noden og holder den til commit, så to rapporter fra samme node ikke
kan committe samtidigt: den anden venter på den førstes skrivning til
transaktionsloggen. Det rammer hårdest en node på vej tilbage fra et udfald,
fordi den eftersender mange rapporter i træk. En række, der ikke matcher
betingelsen, låses aldrig, og de gentagne skrivninger inden for vinduet koster
et opslag i ét indeks og intet andet.

Dataindtaget accepterer komprimering med zstd og med gzip for klienter, der
ikke taler zstd, og svarer selv komprimeret med zstd.


#### REST og webklient

Den anden grænseflade betjener både et JSON-API og webklientens
HTML-fragmenter fra de samme håndteringsfunktioner.

| Modul | Primært formål | Eksempler på stier |
| --- | --- | --- |
| `auth` | Login og session | `/login`, `/logout` |
| `nodes` | Flådens noder og deres tilstand | `/api/nodes`, `/fragments/nodes/{id}` |
| `series` | Tidsserier pr. node, kanal og metrik | `/api/series/{node}/{channel}/{metric}` |
| `alarms` | Alarmer og deres livscyklus | `/api/alarms`, `/api/alarms/{id}/transition` |
| `work_orders` | Udkald og efterprøvning | `/api/work-orders`, `/api/alarms/{id}/dispatch` |
| `admin` | Brugere og nodeadministration | `/api/users`, `/api/nodes/{id}/suspension` |
| `ops` | Systemets egen driftstilstand | `/api/ops/status`, `/api/ops/throughput` |
| `ws` | Realtidshændelser | `/api/ws` |
| `pages` | Sider og sidefragmenter | `/`, `/nodes/{id}`, `/fragments/fleet` |

Hvert modul eksponerer sin egen `routes()`, som samles i `main`. API'et er
desuden dokumenteret maskinlæsbart og kan afprøves direkte gennem Swagger-UI'et
på den publicerede server.


#### Fejlhåndtering

Der er én fejltype, `ApiError`, og én implementering af, hvordan den bliver til
et HTTP-svar. Databasefejl konverteres ind i den, så `?` virker i
håndteringsfunktionerne, og afbildningen til statuskoder er fast:

| Situation | Statuskode |
| --- | --- |
| Manglende eller ugyldig legitimation | 401 |
| Godkendt, men uden rettighed til handlingen | 403 |
| Ukendt ressource | 404 |
| Overtrædelse af en unik nøgle | 409 |
| Overtrædelse af `NOT NULL` eller `CHECK` | 422 |
| Uventet fejl | 500 |

En uventet fejl logges med sin egentlige årsag på serveren og besvares udadtil
med en generisk tekst. Klienten skal vide, at kaldet mislykkedes, ikke hvilken
tabel der er tale om.


#### Realtid

WebSocket-forbindelsen afgør ved oprettelsen, hvilke noder brugeren må høre om,
og filtrerer derefter i hukommelsen. En forbindelse, der kommer bagud, lukkes
aldrig; de beskeder, den er gået glip af, logges og springes over.

Da sessionen kun kontrolleres ved oprettelsen, er en åben socket det eneste i
serveren, der kan blive ved med at svare i timevis uden at blive kontrolleret
igen. Forbindelsen kontrollerer selv sessionen med jævne mellemrum og
lukkes, når den er udløbet.


### Baggrundsarbejde

Serveren kører to baggrundsløkker på hver sin timer. Vedligeholdelsen kører en
gang i timen og flytter mange rækker; detektionen kører hvert minut og læser få.
De er adskilte opgaver, så en partitionsflytning ikke kan forsinke en alarm, og
hver af dem låner kun en forbindelse fra puljen, mens den arbejder, så ingen af
dem tager forbindelser fra dataindtaget.


#### Partitionsvedligeholdelse

Migrationen opretter kun standardpartitionen. Jobbet opretter én partition pr.
døgn i forvejen og dropper dem, der ligger uden for opbevaringshorisonten.

PostgreSQL nægter at oprette en partition for et interval, standardpartitionen
allerede indeholder rækker i, og en `measurements`-tabel uden standardpartition
afviser skrivninger. Jobbet frakobler derfor standardpartitionen, flytter de
rækker, der hører til den nye dag, og tilkobler den igen, alt sammen i én
transaktion. Dataindtaget kører
imens, og en halvt gennemført ændring ville betyde afviste målinger.

Partitionsnavnet udledes udelukkende af datoen, så ingen streng fra en klient
når frem til den DDL, jobbet kører.


#### Fortætning

En node rapporterer ét vindue i minuttet pr. kanal og metrik. Med fem metrikker
og to kanaler er det 14.400 rækker pr. node pr. døgn, og de grafer, operatøren
faktisk kigger på, spænder over måneder. Fortætningen holder de lange visninger
billige uden at indføre en database mere i diagrammet.

Hver opløsning beregnes ud fra de rå rækker og ikke ud fra opløsningen under
den, så længe de rå rækker er inden for deres horisont. Et gennemsnit af
gennemsnit er kun lig gennemsnittet, når grupperne er lige store, og det er
vinduer ikke: en node, der har været nede i en halv time, bidrager med færre
samples til den time end til den næste.

| Størrelse | Overlever | Hvordan |
| --- | --- | --- |
| `min`, `max` | Eksakt | Yderpunkterne af yderpunkterne |
| `sample_count` | Eksakt | Summen |
| `mean` | Eksakt | Vægtet med `sample_count` |
| `stddev` | Eksakt | Gennem identiteten for puljet varians |
| `median` | Tilnærmet | Medianen af vinduernes medianer |
| `p95` | Nej | Kan ikke gendannes af sammendrag |

`p95` er udeladt af `rollups` frem for at stå der som en kolonne, der indeholder
et tal, data ikke understøtter. En percentil af de rå samples kan ikke
rekonstrueres af statistiske sammendrag, og en tilnærmelse, der ikke er markeret
som en tilnærmelse, er værre end en manglende kolonne.


#### Afvigelsesdetektion

Detektoren sammenligner de seneste vinduer med, hvad noden plejer at måle, og
gør det pr. time på døgnet frem for mod et fladt døgngennemsnit. Både
radiobølgeudbredelse og lokal støj følger en daglig cyklus, og et fladt
gennemsnit over 24 timer ville rejse alarm hver nat.

Baselinen bygges over 28 døgn. Centrum er medianen af timegruppernes
gennemsnit. Medianen er robust over for et enkeltstående udfald eller en enkelt
forstyrret eftermiddag, der ellers ville trække et gennemsnit skævt. Spredningen
lægger to bidrag sammen i kvadratur, fordi de er uafhængige: variationen mellem
døgnene for den samme time, målt som median absolute deviation og skaleret med
faktoren 1,4826 til en standardafvigelsesækvivalent, og den typiske spredning
inden for en enkelt time.

Alarmen rejses, når tre på hinanden følgende vinduer ligger uden for båndet på
fire robuste standardafvigelser, og alle tre ligger til samme side. Kravet om
samme side skiller en reel forskydning fra en støjende serie, der rammer begge
sider.

En node skal have mindst syv timegrupper, før den overhovedet kan udløse en
alarm, så en netop installeret node ikke alarmerer i sine første døgn. Og
spredningen har en nedre grænse, fordi en kanal, der har stået på præcis den
samme værdi i 28 døgn, har en median absolute deviation på nul, hvorefter det
næste vindue, der overhovedet afviger, ville rejse alarm.

Lukning sker aldrig automatisk. En alarm forlader kun tilstanden `open` gennem
livscyklus-API'et, hvor et menneske sætter tilstanden og angiver en begrundelse.


### Drift og udrulning


#### Byggeautomatik

Et push til projektets repository udløser en testkørsel, der kompilerer hele
workspace'et, kører den statiske analyse med de samme regler som lokalt og
afvikler den fulde testsuite mod en PostgreSQL-instans, arbejdsgangen selv
starter.

Udrulningen er en selvstændig arbejdsgang, der udløses af, at testkørslen er
færdig, og som kun udfører noget, hvis dens konklusion var `success`. Et push,
der fejler testene, udrulles aldrig. Reglerne for den statiske analyse ligger i
workspace'ets manifest og ikke i en liste af argumenter i arbejdsgangen, så
den samme regel gælder på udviklingsmaskinen og i byggeautomatikken.

Ud over den almindelige testkørsel kan det hele køres lokalt med ét kald,
`nix flake check`, som bygger begge pakker, kører den statiske analyse og
formateringen og afvikler testsuiten mod en kortlivet database, som kørslen selv
starter. Den kræver hverken en kørende databasetjeneste eller netadgang.


#### Værtskonfiguration

Både serveren og noden kører NixOS og konfigureres fra dette repository. Værten,
tjenesten, diskopsætningen og applikationen er beskrevet i den samme kilde, der
bygges og testes, så en udrulning kan ikke afvige fra det, der blev afprøvet.

Det adskiller sig fra de to gængse stakke til Linux-drift.
Docker Compose, der isolerer applikation og database i containere, efterlader
værtsoperativsystemet uden for den kilde, der testes: containeren kan være
korrekt, mens disken, kerneparametre og systemtjenester drives i hånden. Ansible
kan lukke det hul med playbooks, men playbooks muterer en eksisterende maskine
trin for trin og forudsætter et OS; to ens kørsler kan stadig ende forskelligt, hvis
noget uden for playbooken har rørt værten. NixOS erklærer hele systemet som ét
udtryk. Udrulningen er `nixos-rebuild switch` mod en flake-reference,
generationen skiftes atomart, og den forrige ligger klar til rollback.

Caddy står foran applikationen, terminerer TLS og henter certifikater
automatisk. Applikationens to porte, HTTP og gRPC, lytter kun på loopback, når
der er sat et domænenavn, så trafik udefra altid passerer gennem den
terminerende proxy.

Databasens filsystem er sat op uden kopiering ved skrivning. Kopiering ved
skrivning under en database fragmenterer datafilerne og lægger endnu en
skriveforstærkning oven på den, transaktionsloggen allerede har.


#### Overvågning af egen drift

Serveren tæller sin egen aktivitet i hukommelsen: gennemløb i dataindtaget,
svartider, fejlrater og tidspunktet for detektorens seneste gennemførte
gennemløb. Tællerne er driftsdata og ikke historik, og de nulstilles med
processen.

Går der et kvarter, uden at dataindtaget accepterer noget fra nogen, kalder
serveren sig selv degraderet; en node rapporterer én gang i minuttet, så et
kvarter uden en eneste accepteret rapport fra hele flåden er ikke en stille
flåde. Går der tre minutter, uden at detektoren
gennemfører et gennemløb, gælder det samme; det er tre gange dens eget
interval, hvor ét oversprunget interval er en langsom forespørgsel og tre er en
opgave, der har sat sig fast.

En standsning af dataindtaget skrives ikke til `alarms`. Hver række i den
tabel hører til en node, og en server, der er holdt op med at tage imod
skrivninger, er ikke én nodes problem. Tilstanden vises i driftsvisningen i
stedet.
