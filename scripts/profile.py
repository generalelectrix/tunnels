import time
from tunnelz import tunnelz

tunnelz.setup()
n_ave = 10
last = time.time()
n = 0
for _ in xrange(100):
    tunnelz.draw()
    n += 1
    if n % n_ave == 0:

        now = time.time()
        print "{} fps".format(n_ave / (now - last))
        last = now