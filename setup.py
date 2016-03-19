#!/usr/bin/env python
# -*- coding: utf-8 -*-
from Cython.Build import cythonize
try:
    from setuptools import setup, Extension
except ImportError:
    from distutils.core import setup, Extension
import numpy

requires = ['numpy', 'bidict', 'cython', 'msgpack', 'pyzmq', 'rtmidi']

extensions = [
    Extension(
        "tunnelz.waveforms",
        ["tunnelz/waveforms.pyx"],
        include_dirs=[numpy.get_include(), "."],
    )]

setup(
    name='tunnelz',
    packages=['tunnelz'],
    install_requires=requires,
    license='GPL2',
    ext_modules=cythonize(extensions),
)