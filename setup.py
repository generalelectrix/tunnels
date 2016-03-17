#!/usr/bin/env python
# -*- coding: utf-8 -*-

try:
    from setuptools import setup, Extension
except ImportError:
    from distutils.core import setup, Extension

requires = ['numpy', 'scipy', 'protobuf', 'bidict']

setup(
    name='tunnelz',
    install_requires=requires,
    license='GPL2',
    ext_modules=[Extension(
        'draw_commands',
        sources=['cpp/draw_commands.c', 'cpp/draw_commands.pb.cc'],
        libraries=['protobuf'])],
)