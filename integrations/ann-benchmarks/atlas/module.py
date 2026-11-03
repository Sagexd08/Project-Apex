import atlas_ann
import numpy as np

from ..base.module import BaseANN


class Atlas(BaseANN):
    def __init__(self, metric, method_param):
        self.metric = metric
        self.method_param = method_param
        self.index = None
        self.name = "atlas"

    def fit(self, X):
        X = np.ascontiguousarray(X, dtype=np.float32)
        self.index = atlas_ann.Index(
            X.shape[1],
            self.metric,
            self.method_param.get("f16", True),
            self.method_param.get("ef", 128),
        )
        self.index.add(X, 0)

    def set_query_arguments(self, ef):
        self.index.set_ef_search(ef)
        self.name = "atlas (%s, 'efQuery': %s)" % (self.method_param, ef)

    def query(self, v, n):
        return self.index.search(np.ascontiguousarray(v, dtype=np.float32), n)

    def batch_query(self, X, n):
        self.res = self.index.search_batch(np.ascontiguousarray(X, dtype=np.float32), n)

    def get_batch_results(self):
        return self.res

    def freeIndex(self):
        self.index = None
