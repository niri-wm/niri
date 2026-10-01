### AZERTY
When using Niri with an AZERTY keyboard layout, the default workspace keybinds (or any keybinds using numbers) will not work as intended.

#### sn [Wolof], sn(basic) [Wolof], tg [French (Togo)], tg(basic) [French (Togo)], dz [Berber (Algeria, Latin)], dz(azerty-oss) [Berber (Algeria, Latin)], dz(azerty-deadkeys) [Kabyle (AZERTY, with dead keys)], fr [French], fr(basic) [French], fr(olpc) [French], fr(nodeadkeys) [French (no dead keys)], fr(oss) [French (alt.)], fr(oss_latin9) [French (alt., Latin-9 only)], fr(oss_nodeadkeys) [French (alt., no dead keys)], fr(latin9) [French (legacy, alt.)], fr(latin9_nodeadkeys) [French (legacy, alt., no dead keys)], fr(oci) [Occitan], fr(sun_type6), fr(azerty) [French (AZERTY)], cm(french) [French (Cameroon)], ml [Bambara], ml(basic) [Bambara], ml(fr-oss) [French (Mali, alt.)], ma(french) [French (Morocco)], sun_vndr/fr [French (Sun Type 6/7)], sun_vndr/fr(sun_type6) [French (Sun Type 6/7)], sun_vndr/fr(basic), sun_vndr/fr(olpc), sun_vndr/fr(Sundeadkeys), sun_vndr/fr(sundeadkeys), sun_vndr/fr(nodeadkeys), sun_vndr/fr(oss), sun_vndr/fr(oss_latin9), sun_vndr/fr(oss_Sundeadkeys), sun_vndr/fr(oss_sundeadkeys), sun_vndr/fr(oss_nodeadkeys), sun_vndr/fr(latin9), sun_vndr/fr(latin9_Sundeadkeys), sun_vndr/fr(latin9_sundeadkeys), sun_vndr/fr(latin9_nodeadkeys), sun_vndr/fr(oci), ru(phonetic_azerty) [Russian (phonetic, AZERTY)], dz(ar) [Arabic (Algeria)], ara(azerty) [Arabic (AZERTY)], ara(azerty_digits) [Arabic (AZERTY, Eastern Arabic numerals)], ma [Arabic (Morocco)], ma(arabic) [Arabic (Morocco)], sun_vndr/ara(azerty), sun_vndr/ara(azerty_digits)

##### Summary
To fix the above issue, you can replace the following numbers with the key name next to it.

1. ampersand
2. eacute
3. quotedbl
4. apostrophe
5. parenleft
6. minus
7. egrave
8. underscore
9. ccedilla

##### Default bindings translated to Azerty
```kdl
binds {
    // ...

    Mod+ampersand { focus-workspace 1; }
    Mod+eacute { focus-workspace 2; }
    Mod+quotedbl { focus-workspace 3; }
    Mod+apostrophe { focus-workspace 4; }
    Mod+parenleft { focus-workspace 5; }
    Mod+minus { focus-workspace 6; }
    Mod+egrave { focus-workspace 7; }
    Mod+underscore { focus-workspace 8; }
    Mod+ccedilla { focus-workspace 9; }
    Mod+Ctrl+ampersand { move-column-to-workspace 1; }
    Mod+Ctrl+eacute { move-column-to-workspace 2; }
    Mod+Ctrl+quotedbl { move-column-to-workspace 3; }
    Mod+Ctrl+apostrophe { move-column-to-workspace 4; }
    Mod+Ctrl+parenleft { move-column-to-workspace 5; }
    Mod+Ctrl+minus { move-column-to-workspace 6; }
    Mod+Ctrl+egrave { move-column-to-workspace 7; }
    Mod+Ctrl+underscore { move-column-to-workspace 8; }
    Mod+Ctrl+ccedilla { move-column-to-workspace 9; }

    // Alternatively, there are commands to move just a single window:
    // Mod+Ctrl+ampersand { move-window-to-workspace 1; }

    // ...
}
```

#### vn(fr) [Vietnamese (France)], vn(aderty) [Vietnamese (AÐERTY)]

##### Summary
To fix the above issue, you can replace the following numbers with the key name next to it.

1. ampersand
2. dead_tilde
3. quotedbl
4. dead_acute
5. parenleft
6. minus
7. dead_grave
8. underscore
9. ccedilla

##### Default bindings translated to Azerty
```kdl
binds {
    // ...

    Mod+ampersand { focus-workspace 1; }
    Mod+dead_tilde { focus-workspace 2; }
    Mod+quotedbl { focus-workspace 3; }
    Mod+dead_acute { focus-workspace 4; }
    Mod+parenleft { focus-workspace 5; }
    Mod+minus { focus-workspace 6; }
    Mod+dead_grave { focus-workspace 7; }
    Mod+underscore { focus-workspace 8; }
    Mod+ccedilla { focus-workspace 9; }
    Mod+Ctrl+ampersand { move-column-to-workspace 1; }
    Mod+Ctrl+dead_tilde { move-column-to-workspace 2; }
    Mod+Ctrl+quotedbl { move-column-to-workspace 3; }
    Mod+Ctrl+dead_acute { move-column-to-workspace 4; }
    Mod+Ctrl+parenleft { move-column-to-workspace 5; }
    Mod+Ctrl+minus { move-column-to-workspace 6; }
    Mod+Ctrl+dead_grave { move-column-to-workspace 7; }
    Mod+Ctrl+underscore { move-column-to-workspace 8; }
    Mod+Ctrl+ccedilla { move-column-to-workspace 9; }

    // Alternatively, there are commands to move just a single window:
    // Mod+Ctrl+ampersand { move-window-to-workspace 1; }

    // ...
}
```

#### be [Belgian], be(basic) [Belgian], be(oss) [Belgian (alt.)], be(oss_latin9) [Belgian (Latin-9 only, alt.)], be(iso-alternate) [Belgian (ISO, alt.)], be(nodeadkeys) [Belgian (no dead keys)], be(wang) [Belgian (Wang 724 AZERTY)], be(sun_type6), fr(mac) [French (Macintosh)], sun_vndr/be [Belgium (Sun Type 6/7)], sun_vndr/be(sun_type6) [Belgium (Sun Type 6/7)], sun_vndr/be(basic), sun_vndr/be(oss), sun_vndr/be(oss_latin9), sun_vndr/be(iso-alternate), sun_vndr/be(nodeadkeys), sun_vndr/be(wang), sun_vndr/fr(mac), macintosh_vndr/fr [French (Macintosh)], macintosh_vndr/fr(extended) [French (Macintosh)], macintosh_vndr/fr(nodeadkeys) [French (Macintosh, no dead keys)]

##### Summary
To fix the above issue, you can replace the following numbers with the key name next to it.

1. ampersand
2. eacute
3. quotedbl
4. apostrophe
5. parenleft
6. section
7. egrave
8. exclam
9. ccedilla

##### Default bindings translated to Azerty
```kdl
binds {
    // ...

    Mod+ampersand { focus-workspace 1; }
    Mod+eacute { focus-workspace 2; }
    Mod+quotedbl { focus-workspace 3; }
    Mod+apostrophe { focus-workspace 4; }
    Mod+parenleft { focus-workspace 5; }
    Mod+section { focus-workspace 6; }
    Mod+egrave { focus-workspace 7; }
    Mod+exclam { focus-workspace 8; }
    Mod+ccedilla { focus-workspace 9; }
    Mod+Ctrl+ampersand { move-column-to-workspace 1; }
    Mod+Ctrl+eacute { move-column-to-workspace 2; }
    Mod+Ctrl+quotedbl { move-column-to-workspace 3; }
    Mod+Ctrl+apostrophe { move-column-to-workspace 4; }
    Mod+Ctrl+parenleft { move-column-to-workspace 5; }
    Mod+Ctrl+section { move-column-to-workspace 6; }
    Mod+Ctrl+egrave { move-column-to-workspace 7; }
    Mod+Ctrl+exclam { move-column-to-workspace 8; }
    Mod+Ctrl+ccedilla { move-column-to-workspace 9; }

    // Alternatively, there are commands to move just a single window:
    // Mod+Ctrl+ampersand { move-window-to-workspace 1; }

    // ...
}
```

#### fr(us-azerty) [French (US, AZERTY)]

##### Summary
To fix the above issue, you can replace the following numbers with the key name next to it.

1. 1
2. 2
3. 3
4. 4
5. 5
6. 6
7. 7
8. 8
9. 9

##### Default bindings translated to Azerty
```kdl
binds {
    // ...

    Mod+1 { focus-workspace 1; }
    Mod+2 { focus-workspace 2; }
    Mod+3 { focus-workspace 3; }
    Mod+4 { focus-workspace 4; }
    Mod+5 { focus-workspace 5; }
    Mod+6 { focus-workspace 6; }
    Mod+7 { focus-workspace 7; }
    Mod+8 { focus-workspace 8; }
    Mod+9 { focus-workspace 9; }
    Mod+Ctrl+1 { move-column-to-workspace 1; }
    Mod+Ctrl+2 { move-column-to-workspace 2; }
    Mod+Ctrl+3 { move-column-to-workspace 3; }
    Mod+Ctrl+4 { move-column-to-workspace 4; }
    Mod+Ctrl+5 { move-column-to-workspace 5; }
    Mod+Ctrl+6 { move-column-to-workspace 6; }
    Mod+Ctrl+7 { move-column-to-workspace 7; }
    Mod+Ctrl+8 { move-column-to-workspace 8; }
    Mod+Ctrl+9 { move-column-to-workspace 9; }

    // Alternatively, there are commands to move just a single window:
    // Mod+Ctrl+1 { move-window-to-workspace 1; }

    // ...
}
```

#### fr(afnor) [French (AZERTY, AFNOR)]

##### Summary
To fix the above issue, you can replace the following numbers with the key name next to it.

1. agrave
2. eacute
3. egrave
4. ecircumflex
5. parenleft
6. parenright
7. leftsinglequotemark
8. rightsinglequotemark
9. guillemotleft

##### Default bindings translated to Azerty
```kdl
binds {
    // ...

    Mod+agrave { focus-workspace 1; }
    Mod+eacute { focus-workspace 2; }
    Mod+egrave { focus-workspace 3; }
    Mod+ecircumflex { focus-workspace 4; }
    Mod+parenleft { focus-workspace 5; }
    Mod+parenright { focus-workspace 6; }
    Mod+leftsinglequotemark { focus-workspace 7; }
    Mod+rightsinglequotemark { focus-workspace 8; }
    Mod+guillemotleft { focus-workspace 9; }
    Mod+Ctrl+agrave { move-column-to-workspace 1; }
    Mod+Ctrl+eacute { move-column-to-workspace 2; }
    Mod+Ctrl+egrave { move-column-to-workspace 3; }
    Mod+Ctrl+ecircumflex { move-column-to-workspace 4; }
    Mod+Ctrl+parenleft { move-column-to-workspace 5; }
    Mod+Ctrl+parenright { move-column-to-workspace 6; }
    Mod+Ctrl+leftsinglequotemark { move-column-to-workspace 7; }
    Mod+Ctrl+rightsinglequotemark { move-column-to-workspace 8; }
    Mod+Ctrl+guillemotleft { move-column-to-workspace 9; }

    // Alternatively, there are commands to move just a single window:
    // Mod+Ctrl+agrave { move-window-to-workspace 1; }

    // ...
}
```

#### cm(azerty) [Cameroon (AZERTY, intl.)]

##### Summary
To fix the above issue, you can replace the following numbers with the key name next to it.

1. U0026
2. eacute
3. U0022
4. U0027
5. U0028
6. U002D
7. U00E8
8. underscore
9. ccedilla

##### Default bindings translated to Azerty
```kdl
binds {
    // ...

    Mod+U0026 { focus-workspace 1; }
    Mod+eacute { focus-workspace 2; }
    Mod+U0022 { focus-workspace 3; }
    Mod+U0027 { focus-workspace 4; }
    Mod+U0028 { focus-workspace 5; }
    Mod+U002D { focus-workspace 6; }
    Mod+U00E8 { focus-workspace 7; }
    Mod+underscore { focus-workspace 8; }
    Mod+ccedilla { focus-workspace 9; }
    Mod+Ctrl+U0026 { move-column-to-workspace 1; }
    Mod+Ctrl+eacute { move-column-to-workspace 2; }
    Mod+Ctrl+U0022 { move-column-to-workspace 3; }
    Mod+Ctrl+U0027 { move-column-to-workspace 4; }
    Mod+Ctrl+U0028 { move-column-to-workspace 5; }
    Mod+Ctrl+U002D { move-column-to-workspace 6; }
    Mod+Ctrl+U00E8 { move-column-to-workspace 7; }
    Mod+Ctrl+underscore { move-column-to-workspace 8; }
    Mod+Ctrl+ccedilla { move-column-to-workspace 9; }

    // Alternatively, there are commands to move just a single window:
    // Mod+Ctrl+U0026 { move-window-to-workspace 1; }

    // ...
}
```

#### cd [French (Democratic Republic of the Congo)], cd(basic) [French (Democratic Republic of the Congo)]

##### Summary
To fix the above issue, you can replace the following numbers with the key name next to it.

1. ampersand
2. U0301
3. U0300
4. parenleft
5. braceleft
6. braceright
7. parenright
8. U0302
9. U030C

##### Default bindings translated to Azerty
```kdl
binds {
    // ...

    Mod+ampersand { focus-workspace 1; }
    Mod+U0301 { focus-workspace 2; }
    Mod+U0300 { focus-workspace 3; }
    Mod+parenleft { focus-workspace 4; }
    Mod+braceleft { focus-workspace 5; }
    Mod+braceright { focus-workspace 6; }
    Mod+parenright { focus-workspace 7; }
    Mod+U0302 { focus-workspace 8; }
    Mod+U030C { focus-workspace 9; }
    Mod+Ctrl+ampersand { move-column-to-workspace 1; }
    Mod+Ctrl+U0301 { move-column-to-workspace 2; }
    Mod+Ctrl+U0300 { move-column-to-workspace 3; }
    Mod+Ctrl+parenleft { move-column-to-workspace 4; }
    Mod+Ctrl+braceleft { move-column-to-workspace 5; }
    Mod+Ctrl+braceright { move-column-to-workspace 6; }
    Mod+Ctrl+parenright { move-column-to-workspace 7; }
    Mod+Ctrl+U0302 { move-column-to-workspace 8; }
    Mod+Ctrl+U030C { move-column-to-workspace 9; }

    // Alternatively, there are commands to move just a single window:
    // Mod+Ctrl+ampersand { move-window-to-workspace 1; }

    // ...
}
```

#### fr(geo) [Georgian (France, AZERTY Tskapo)], sun_vndr/fr(geo)

##### Summary
To fix the above issue, you can replace the following numbers with the key name next to it.

1. U201E
2. U2116
3. percent
4. parenleft
5. colon
6. semicolon
7. question
8. U2116
9. degree

##### Default bindings translated to Azerty
```kdl
binds {
    // ...

    Mod+U201E { focus-workspace 1; }
    Mod+U2116 { focus-workspace 2; }
    Mod+percent { focus-workspace 3; }
    Mod+parenleft { focus-workspace 4; }
    Mod+colon { focus-workspace 5; }
    Mod+semicolon { focus-workspace 6; }
    Mod+question { focus-workspace 7; }
    Mod+U2116 { focus-workspace 8; }
    Mod+degree { focus-workspace 9; }
    Mod+Ctrl+U201E { move-column-to-workspace 1; }
    Mod+Ctrl+U2116 { move-column-to-workspace 2; }
    Mod+Ctrl+percent { move-column-to-workspace 3; }
    Mod+Ctrl+parenleft { move-column-to-workspace 4; }
    Mod+Ctrl+colon { move-column-to-workspace 5; }
    Mod+Ctrl+semicolon { move-column-to-workspace 6; }
    Mod+Ctrl+question { move-column-to-workspace 7; }
    Mod+Ctrl+U2116 { move-column-to-workspace 8; }
    Mod+Ctrl+degree { move-column-to-workspace 9; }

    // Alternatively, there are commands to move just a single window:
    // Mod+Ctrl+U201E { move-window-to-workspace 1; }

    // ...
}
```

#### gn [N'Ko (AZERTY)], gn(basic) [N'Ko (AZERTY)]

##### Summary
To fix the above issue, you can replace the following numbers with the key name next to it.

1. U07F1
2. U07EB
3. U07F5
4. U07F4
5. parenleft
6. minus
7. U07EC
8. U07FA
9. U07ED

##### Default bindings translated to Azerty
```kdl
binds {
    // ...

    Mod+U07F1 { focus-workspace 1; }
    Mod+U07EB { focus-workspace 2; }
    Mod+U07F5 { focus-workspace 3; }
    Mod+U07F4 { focus-workspace 4; }
    Mod+parenleft { focus-workspace 5; }
    Mod+minus { focus-workspace 6; }
    Mod+U07EC { focus-workspace 7; }
    Mod+U07FA { focus-workspace 8; }
    Mod+U07ED { focus-workspace 9; }
    Mod+Ctrl+U07F1 { move-column-to-workspace 1; }
    Mod+Ctrl+U07EB { move-column-to-workspace 2; }
    Mod+Ctrl+U07F5 { move-column-to-workspace 3; }
    Mod+Ctrl+U07F4 { move-column-to-workspace 4; }
    Mod+Ctrl+parenleft { move-column-to-workspace 5; }
    Mod+Ctrl+minus { move-column-to-workspace 6; }
    Mod+Ctrl+U07EC { move-column-to-workspace 7; }
    Mod+Ctrl+U07FA { move-column-to-workspace 8; }
    Mod+Ctrl+U07ED { move-column-to-workspace 9; }

    // Alternatively, there are commands to move just a single window:
    // Mod+Ctrl+U07F1 { move-window-to-workspace 1; }

    // ...
}
```

