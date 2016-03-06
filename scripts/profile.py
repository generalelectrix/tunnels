import time
from tunnelz import tunnelz

def run():
    tunnelz.setup()
    n_ave = 10
    last = time.time()
    for n in xrange(100):
        tunnelz.draw(write=False)
        if (n + 1) % n_ave == 0:

            now = time.time()
            print "{} fps".format(n_ave / (now - last))
            last = now

if __name__ == "__main__":
    run()