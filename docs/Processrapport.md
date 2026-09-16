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


## Rust

Rust er et systemprogrammeringssprog, der kompileres til maskinkode uden en
mellemliggende virtuel maskine. Det adskiller sig fra C og C++ ved, at
hukommelsessikkerheden kontrolleres af compileren i stedet for af
programmøren, og fra Java og C# ved, at den kontrol sker ved kompilering i
stedet for af en garbage collector under kørslen. Ejerskabsmodellen afgør ved
kompilering, hvornår en værdi frigives, og der er ingen oprydningsfase, hvis
tidspunkt køretiden bestemmer.

Både serveren og node-agenten er skrevet i Rust, og protokollen ligger som en
tredje pakke, begge kompilerer imod. Feltnavne, opregnede typer og
værdiintervaller findes ét sted, og en ændring i skemaet bryder kompileringen
på begge sider i stedet for at dukke op som mærkelige data i driften.

Fraværet af en garbage collector afgør valget på noden. Signalkæden regner inde
i en måleløkke, som læser fra modtageren i faste blokke. En oprydningsfase, hvis
tidspunkt bestemmes af køretiden og ikke af koden, lander midt i den løkke og
taber samples. Det sekundære argument er fejlhåndteringen: en operation, der kan
fejle, returnerer `Result`, og en ubehandlet fejl bliver en kompileringsfejl og
ikke en hændelse i drift.

C# med ASP.NET Core blev fravalgt. Økosystemet er modent, web-delen ville have
været hurtigere at bygge, og meget af det, der her er skrevet i hånden, findes
færdigt i .NET. CLR'en rydder dog op med en garbage collector, og nodens samplesti
tåler ikke dens uforudsigelige pauser. Exceptions kan
kastes fra ethvert punkt og propagere op gennem kaldstakken uden at optræde i en
signatur, så en fejlsti bliver usynlig for den, der læser koden. Og .NET på
noden ville betyde en køretid ved siden af signalkæden på en maskine, der i
forvejen er dimensioneret til at have hovedrum og ikke rigeligt.

Prisen for Rust betales i kompileringstid. Fuld LTO og én kodegenereringsenhed
gør et koldt release-build af hele workspace'et langsommere, og indstillingerne
er beholdt, fordi de gør node-agentens binære fil ~40% mindre.


## axum og tokio

axum er et asynkront web-framework til Rust. Det er bygget på tokio, sprogets
mest udbredte asynkrone køretid, og på tower, der sammensætter netværkstjenester
af genbrugelige mellemled. Et endepunkt er en almindelig funktion, der tager sine
parametre som udtrækkere og returnerer en typefast respons, så et svar i forkert
format er en kompileringsfejl.

axum bærer REST-API'et, HTML-siderne og WebSocket-forbindelsen. Serveren hviler
på én fejltype med én `IntoResponse`, en `AuthUser`-udtrækker, en
broadcast-kanal til åbne sockets og integrationstest, der driver routeren
direkte uden at starte en HTTP-server.

Det tungeste argument ligger i gRPC-delen. `tonic` router selv sine kald gennem
`axum::Router`, så dataindtaget og webklienten deler køretid, HTTP-lag og
tower-mellemled, og REST-siden lægger ingen ekstra HTTP-stak ind i den binære
fil.

Actix-Web blev fravalgt, fordi dens aktørmodel lægger et ekstra lag mellem
forespørgsel og håndtering, som dette projekt ikke har brug for. Rocket blev
fravalgt, fordi dens afhængighed af ustabile sprogfunktioner historisk har gjort
versionsopgraderinger dyrere end nødvendigt for en tjeneste af denne størrelse.


## tonic og prost

Protocol Buffers er et binært dataformat, hvor beskedernes felter og typer
beskrives i en skemafil, og gRPC er den fjernkaldsprotokol, der sender dem over
HTTP/2. Skemaet er kontrakten: en compiler læser `.proto`-filen og udskriver
både klienten og tjenesten i det sprog, kalderen arbejder i.

Dataindtaget tales over gRPC. `tonic` er tjenesten, `prost` er
kodegenereringen. Protokolpakken er en selvstændig crate, så server, node-agent
og en tredjepart genererer deres kode af det samme skema.

Valget følger direkte af K1. Kravet er et maskinlæsbart skema, der kan
offentliggøres uafhængigt af kildekoden, med entydig afvisning af ukendte
versioner og mulighed for at betjene flere versioner sideløbende. Et
protobuf-skema med versionsfelt i pakkenavnet opfylder det uden en
hjemmelavet kontraktsprotokol oven på JSON.

JSON over REST blev fravalgt til nodens dataindtag. Det kunne have samlet begge
grænseflader på ét transportlag, men det ville have svækket skemaet som kontrakt: en
JSON-kontrakt er dokumentation, ikke en generérbar klient, og
versionsforhandling bliver et konventionsspørgsmål i stedet for et
pakkespørgsmål.


## diesel og PostgreSQL

Databasen er PostgreSQL. Adgangen fra Rust går gennem diesel over en
forbindelsespulje, og migrationerne versioneres sammen med kildekoden og køres
af serveren ved opstart.

Databasebibliotekerne i Rust falder i tre grupper. Den første sender rå
SQL-strenge, hvor en syntaksfejl først viser sig ved kørsel. Den anden bygger
forespørgslen med en forespørgselsbygger uden fuld typekontrol mod skemaet. Den tredje
verificerer forespørgslen ved kompilering, og der ligger både diesel og SQLx.

De to verificerer forskelligt. SQLx validerer SQL-strenge gennem makroer og
kræver derfor adgang til en levende database under kompileringen. Diesel går den
modsatte vej: forespørgslen skrives som Rust-typer, og skemaet er en genereret
Rust-fil, hvorefter compileren afviser et kolonnenavn, der ikke findes, og en
type, der ikke passer. Diesel blev valgt, fordi `nix flake check` bygger og
tester hele workspace'et uden netadgang og selv starter sin database undervejs.
SQLx' krav om en database allerede ved kompilering passer ikke ind i den kørsel.

Tidsserielagringen er almindelig PostgreSQL med egne partitioner og eget
fortætningsjob. TimescaleDB blev fravalgt. K4 kræver en aldersbestemt
opbevaringspolitik og en fortætning af rå målinger til grovere
opløsning, og udvidelsen ville flytte det krav ud i tredjepartskode. Dertil er
udvidelsen en binær komponent, der skal være installeret på værten i en version,
der passer til serverens: endnu en version at holde styr på i en opsætning, der
ellers er låst af `flake.lock`. ClickHouse blev fravalgt, fordi måledata så ville
ligge i et andet system end noderne og kanalerne, de peger på, og uden
fremmednøglen mellem `measurements` og `nodes` kan en måling blive forældreløs.


## spektra-fft

Frekvenstransformationen under Welch-estimeringen er en radix-2
Cooley-Tukey-FFT, skrevet som en del af dette arbejde og udgivet som den
selvstændige pakke `spektra-fft`. Kildekoden ligger på
<https://github.com/BastianAsmussen/spektra-fft>.

K2 kræver, at signalkæden implementeres i projektet, og
frekvenstransformationen er kædens første trin. `rustfft` er det modne valg og er
hurtigere, men et opkald til den ville have flyttet det trin ud af kæden og ind
i en afhængighed.

Forskellen i hastighed ligger i to ting, `spektra-fft` ikke har: håndskrevne
SIMD-kerner og andre radixer end 2. Begge dele blev skåret af tidsplanen, og
ingen af dem er nødvendige ved den skala, systemet kører på.
Effektspektrumsestimeringen står for knap en tredjedel af kædens regnetid, og
noden har hovedrum til den over fire kanaler.

Udskillelsen i en separat crate ændrer intet ved koden. En transformation er
brugbar langt uden for dette system, og som selvstændig pakke kan den
vedligeholdes for sig.


## askama, HTMX og Tailwind

askama er en skabelonmotor, der kompilerer HTML-skabelonerne sammen med resten
af serveren, så en skabelon, der bruger et felt, som ikke findes, giver en
kompileringsfejl og ikke et tomt felt i brugerfladen. HTMX er et
JavaScript-bibliotek, der lader en HTML-attribut sende en HTTP-anmodning og
indsætte svaret i et navngivet element. Tailwind er et CSS-framework, hvor stilen
skrives som småklasser direkte i markup'en, og hvor kun de klasser, projektet
faktisk bruger, ender i den byggede fil.

Webklienten er serverrenderet: serveren sender HTML-fragmenter, og
klienten indsætter dem uden at genindlæse siden.

Valget går imod en klienttung enkelt-sides applikation. En operativ
overvågningsflade har brug for, at en alarm kan skubbes ind i en åben side, og
at adressen på en åben node kan sendes til en kollega. Det løses med
serverrenderede fragmenter og rigtige ruter, uden at vedligeholde en separat
JavaScript-applikation.

SolidJS og React blev fravalgt, fordi de flytter gengivelsen til browseren og
introducerer et build-trin, projektet ikke ellers har. Leptos og Yew blev
prøvet i små eksperimenter og fravalgt, fordi Tailwind-integrationen og det
øvrige økosystem omkring serverrenderede fragmenter var svagere end HTMX til den
type flade.


## uPlot og Leaflet

uPlot er et diagrambibliotek, der tegner på et `canvas`-element i stedet for at
bygge DOM-noder, og Leaflet er et kortbibliotek, der lægger markører og
felter oven på fliser fra en kortudbyder. Begge er lagt ind i projektet som
filer og hentes ikke fra et indholdsleveringsnetværk, så webklientens kode ikke
afhænger af, at en tredjeparts server svarer. Kun kortfliserne hentes udefra.

uPlot blev valgt for `canvas`-tegningen. En tidsserie over en måned er tusindvis
af punkter, og et bibliotek, der giver hvert punkt sin egen DOM-node, gør
browseren til flaskehalsen længe før serveren bliver det. Chart.js blev fravalgt,
fordi det bliver tungt på lange serier, og D3 blev fravalgt, fordi det er et
generelt visualiseringsværktøj, hvor et tidsseriediagram skal bygges af
primitiver og ikke bare konfigureres.

OpenLayers var et reelt alternativ til Leaflet med projektioner, vektorformater
og lagstyring, dog er det et fuldt GIS-bibliotek, og et kort med én markør pr.
node bruger ingen af de dele. MapLibre GL tegner vektorfliser med WebGL og
klarer langt flere punkter, dog kræver det både WebGL i browseren og en
vektorflisekilde med tilhørende stilark, hvor Leaflet nøjes med almindelige
kortfliser fra OpenStreetMap.


## ntfy

ntfy er en notifikationstjeneste, hvor et emne er en URL, og hvor en besked
sendes med en almindelig HTTP-POST til den. En modtager abonnerer på emnet fra
en browser eller fra ntfy's egen app, og der kræves hverken konto eller
registrering hos en platformsudbyder. Serveren er én binær fil, som kan hostes
sammen med resten.

Serveren sender nye alarmer gennem den, når et emne er konfigureret. E-mail blev fravalgt som primær
kanal, fordi leveringstid og spamfiltrering gør den uegnet til en alarm, der
skal ses inden for sekunder. Webhooks blev fravalgt som eneste kanal, fordi de
forudsætter, at modtageren allerede har et system til at tage imod dem; en
tekniker med en telefon har det ikke. Push gennem Firebase Cloud Messaging
ville have krævet en mobilapplikation, og systemet har ikke nogen.


## NixOS og Caddy

NixOS er en Linux-distribution, hvor maskinens tilstand er deklareret frem for
konfigureret. Systemet beskrives som moduler, hvert med sit ansvarsområde, og de
evalueres til ét samlet udtryk, som kerne, tjenester, brugere, diske og
applikationsbinærer alle udledes af. En opdatering bygger den nye generation ved
siden af den kørende og skifter til den i ét trin. Caddy er en webserver, der
henter og fornyer TLS-certifikater af sig selv, når den kender værtens
domænenavn.

Både den centrale server og noden kører NixOS og konfigureres fra det samme
repository som applikationen. Caddy står foran serveren og terminerer TLS.

Docker Compose bag en reverse proxy på en manuelt konfigureret VPS blev
fravalgt: et reproducerbart, isoleret miljø for applikation og database, hvor
containeren bærer køretid og afhængigheder, mens værtsoperativsystemet ligger
udenfor. Ansible blev ligeledes fravalgt: playbooks, der muterer en eksisterende
maskine til den ønskede tilstand. Begge adskiller værtskonfigurationen fra den
kilde, der bygges og testes.

Her er værten, tjenesten, diskopsætningen og applikationen deklareret i samme
repository. `nix flake check` bygger og tester uden netadgang, og udrulningen er
`nixos-rebuild switch` mod en flake-reference, betinget af at testkørslen er
grøn. En generation skiftes atomart; der er ingen playbook, der kan gå i stå
halvvejs, og ingen container, der kører korrekt oven på et OS, hvis
konfiguration har flyttet sig.

Et Docker-image bygges oven på et foranderligt basisimage; en flake er låst af
`flake.lock`. En compose-fil beskriver containere, ikke disken under databasen;
her er filsystemet uden kopiering ved skrivning erklæret sammen med tjenesten.
Og testene kører gennem flaken selv: `nix flake check` starter sin egen
PostgreSQL uden netadgang, og der er intet ekstra script ved siden af.

Ansible tabte på en garanti, playbooks ikke kan give. En playbook forudsætter et
OS og nærmer sig den ønskede tilstand trin for trin; to kørsler med samme
playbook kan stadig lande forskelligt, hvis noget uden for playbooken har rørt
maskinen. NixOS erklærer hele systemet som ét udtryk. Efter `nixos-rebuild
switch` kører den generation, flaken peger på, inklusive kerne, tjenester og
applikationsbinær, og den forrige generation ligger klar til rollback.
