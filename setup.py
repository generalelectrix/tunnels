#!/usr/bin/env python
# -*- coding: utf-8 -*-

try:
    from setuptools import setup
except ImportError:
    from distutils.core import setup

requires = ['numpy', 'bidict']

setup(
    name='tunnelz',
    install_requires=requires,
    license='GPL2',
)