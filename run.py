from tunnelz import Show
print(r"""

                  )     )       (        )
  *   )        ( /(  ( /(       )\ )  ( /(
` )  /(    (   )\()) )\()) (   (()/(  )\())
 ( )(_))   )\ ((_)\ ((_)\  )\   /(_))((_)\
(_(_()) _ ((_) _((_) _((_)((_) (_))   _((_)
|_   _|| | | || \| || \| || __|| |   |_  /
  | |  | |_| || .` || .` || _| | |__  / /
  |_|   \___/ |_|\_||_|\_||___||____|/___|


CONFIGURATION
""")
show = Show.from_prompt()
print("Show loaded, bound to 'show' in interpreter.")
show.run()